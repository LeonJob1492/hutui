//! 极简 CDP 客户端：WebSocket + JSON-RPC。
//!
//! 只做两件事：连上一个真实的 Chrome，然后在页面上下文里发 `fetch` 取 JSON。
//! 不模拟点击、不模拟滚动、不解析 DOM。

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    responses: HashMap<u64, Value>,
    /// 连接或目标已死，需要上层重建会话
    dead: bool,
}

fn discover_ws_url(port: u16) -> Result<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .context("连不上 Chrome 调试端口")?;
    s.set_read_timeout(Some(Duration::from_millis(300)))?;

    let req = format!(
        "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes())?;

    // 不能依赖 read_to_string：Chrome 不一定会主动关连接，
    // 读到 EOF 会变成超时错误。改成「慢慢读、能解析就停」。
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut body_start: Option<usize> = None;

    while Instant::now() < deadline {
        match s.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if body_start.is_none() {
                    body_start = raw.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                }
                if let Some(p) = body_start {
                    if serde_json::from_slice::<Value>(&raw[p..]).is_ok() {
                        break;
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.into()),
        }
    }

    let start = body_start.ok_or_else(|| anyhow!("调试端口没返回合法 HTTP 响应"))?;
    let v: Value = serde_json::from_slice(&raw[start..]).context("解析 /json/version 失败")?;
    v["webSocketDebuggerUrl"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("调试端口没返回 webSocketDebuggerUrl"))
}

fn brief(url: &str) -> String {
    url.splitn(2, "zhihu.com").nth(1).unwrap_or(url).chars().take(80).collect()
}

impl Cdp {
    pub fn connect(port: u16) -> Result<Self> {
        let url = discover_ws_url(port)?;
        let (mut ws, _) = tungstenite::connect(&url).context("连接 CDP WebSocket 失败")?;
        if let MaybeTlsStream::Plain(s) = ws.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(100)))?;
        }
        Ok(Self {
            ws,
            next_id: 0,
            responses: HashMap::new(),
            dead: false,
        })
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    fn push(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        if let Err(e) = self.ws.send(Message::Text(msg.to_string())) {
            // 对端已关闭（AlreadyClosed 之类）也要算死，不能只靠后面的 pump 兜
            self.dead = true;
            return Err(e).context("发送 CDP 消息失败");
        }
        Ok(id)
    }

    fn absorb(&mut self, raw: &str) {
        let Ok(v) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            self.responses.insert(id, v);
        }
    }

    fn pump(&mut self, dur: Duration) -> Result<()> {
        let deadline = Instant::now() + dur;
        while Instant::now() < deadline {
            match self.ws.read() {
                Ok(Message::Text(t)) => self.absorb(&t),
                Ok(Message::Binary(b)) => {
                    if let Ok(s) = std::str::from_utf8(&b) {
                        let owned = s.to_string();
                        self.absorb(&owned);
                    }
                }
                Ok(Message::Close(_)) => {
                    self.dead = true;
                    bail!("CDP 连接被对端关闭")
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(tungstenite::Error::ConnectionClosed) => {
                    self.dead = true;
                    bail!("CDP 连接已断开")
                }
                Err(e) => {
                    self.dead = true;
                    return Err(e.into())
                }
            }
        }
        Ok(())
    }

    pub fn call(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.push(method, params, session)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(v) = self.responses.remove(&id) {
                if let Some(err) = v.get("error") {
                    let text = err.to_string();
                    // 标签页被关、会话失效这类错误：标记成死，交给上层重连
                    for marker in [
                        "Target closed",
                        "Session with given id not found",
                        "Inspected target navigated or closed",
                        "No target with given id",
                        "Not attached to an active page",
                    ] {
                        if text.contains(marker) {
                            self.dead = true;
                            break;
                        }
                    }
                    bail!("CDP {method} 出错: {err}");
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            if Instant::now() > deadline {
                // Chrome 卡死时别让后面每条命令都再等 30s
                self.dead = true;
                bail!("CDP {method} 超时");
            }
            self.pump(Duration::from_millis(30))?;
        }
    }

    /// 在页面上下文里跑一段 JS，返回 (ok, status, text)。
    /// 顺带把「CDP 往返」和「页面内 fetch」两段耗时写进 trace.log。
    fn run_js(&mut self, session: &str, label: &str, expr: &str) -> Result<(bool, i64, String)> {
        let t0 = Instant::now();
        let res = self.call(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }),
            Some(session),
        )?;
        let cdp = t0.elapsed();

        // promise 被 reject 时 CDP 会带 exceptionDetails，而 result.value 可能是 {}
        // —— 不先看这里，就会把"脚本炸了"误读成"HTTP 0 空响应"。
        if let Some(details) = res.get("exceptionDetails") {
            let desc = details
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("页面脚本抛异常");
            let brief: String = desc.lines().next().unwrap_or(desc).chars().take(120).collect();
            bail!("页面脚本出错: {brief}");
        }

        let val = &res["result"]["value"];
        if val.is_null() {
            bail!("页面求值失败: 没有返回值");
        }
        // 用同一台机器的墙钟拆三段：派发（CDP→渲染进程）、fetch、回传（渲染进程→我们）
        let fetch_ms = val["ms"].as_f64().unwrap_or(-1.0);
        let t_out = val["tout"].as_i64().unwrap_or(0);
        let t_recv = crate::trace::epoch_ms();
        let deliver = if t_out > 0 { t_recv - t_out } else { -1 };
        crate::trace::log(&format!(
            "{label} cdp={} fetch={fetch_ms:.0}ms deliver={deliver}ms",
            crate::trace::ms(cdp)
        ));

        Ok((
            val["ok"].as_bool().unwrap_or(false),
            val["status"].as_i64().unwrap_or(0),
            val["text"].as_str().unwrap_or("").to_string(),
        ))
    }

    /// 读接口：在页面同源上下文里 GET，返回解析后的 JSON。
    ///
    /// 用的是页面自己的 cookie —— 不做任何签名伪造，这些接口本身也不校验签名。
    pub fn fetch_json(&mut self, session: &str, url: &str) -> Result<Value> {
        let expr = fetch_expr(url)?;

        let label = format!("GET {}", brief(url));
        let (ok, status, text) = self.run_js(session, &label, &expr)?;
        let head: String = text.chars().take(160).collect();
        if status == 0 {
            bail!("{} → 请求没发出去: {head}", brief(url));
        }
        if !ok || status != 200 {
            bail!("{} → HTTP {status}: {head}", brief(url));
        }
        serde_json::from_str(&text).with_context(|| format!("{} 返回的不是 JSON", brief(url)))
    }

    /// 写接口：带上页面里的 `_xsrf` 作为 x-xsrftoken。
    pub fn write_json(
        &mut self,
        session: &str,
        method: &str,
        url: &str,
        body: Option<&str>,
    ) -> Result<Value> {
        let expr = write_expr(method, url, body)?;

        let label = format!("{method} {}", brief(url));
        let (ok, status, text) = self.run_js(session, &label, &expr)?;
        let head: String = text.chars().take(160).collect();
        if status == 0 {
            bail!("{method} {} → 请求没发出去: {head}", brief(url));
        }
        if !ok || !(200..300).contains(&status) {
            bail!("{method} {} → HTTP {status}: {head}", brief(url));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

/// 生成「读接口」的页面脚本。抽成纯函数是为了能被测试盯住——
/// 模板里引用未声明变量会变成 `ReferenceError`，而 V8 会把 reject 值序列化成 `{}`，
/// 上层就会误读成 `HTTP 0` 空响应，排查起来很费劲。
fn fetch_expr(url: &str) -> Result<String> {
    let quoted = serde_json::to_string(url)?;
    Ok(format!(
        "(async () => {{ \
           const tin = Date.now(), t0 = performance.now(); \
           try {{ \
             const r = await fetch({quoted}, {{ credentials: 'include' }}); \
             const text = await r.text(); \
             return {{ ok: r.ok, status: r.status, text, ms: performance.now() - t0, \
                       tin, tout: Date.now() }}; \
           }} catch (e) {{ return {{ ok: false, status: 0, text: String(e), ms: -1, \
                                    tin, tout: Date.now() }}; }} \
         }})()"
    ))
}

/// 生成「写接口」的页面脚本（带 x-xsrftoken）
fn write_expr(method: &str, url: &str, body: Option<&str>) -> Result<String> {
    let url_lit = serde_json::to_string(url)?;
    let method_lit = serde_json::to_string(method)?;
    let body_lit = match body {
        Some(b) => serde_json::to_string(b)?,
        None => "null".to_string(),
    };
    Ok(format!(
        "(async () => {{ \
           const xsrf = decodeURIComponent(((document.cookie.match(/_xsrf=([^;]+)/) || [])[1] || '')); \
           const tin = Date.now(), t0 = performance.now(); \
           try {{ \
             const h = {{ 'x-xsrftoken': xsrf }}; \
             if ({body_lit} !== null) h['content-type'] = 'application/json'; \
             const r = await fetch({url_lit}, {{ \
               method: {method_lit}, \
               credentials: 'include', \
               headers: h, \
               body: {body_lit} \
             }}); \
             const text = await r.text(); \
             return {{ ok: r.ok, status: r.status, text, ms: performance.now() - t0, \
                       tin, tout: Date.now() }}; \
           }} catch (e) {{ return {{ ok: false, status: 0, text: String(e), ms: -1, \
                                    tin, tout: Date.now() }}; }} \
         }})()"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模板里每个用到的计时变量都必须先声明；写错过一次 `t_in`，
    /// 表现为 `HTTP 0:`，很难从客户端看出来。
    #[test]
    fn expressions_declare_every_timing_var() {
        for expr in [
            fetch_expr("https://www.zhihu.com/api/v4/me").unwrap(),
            write_expr("POST", "https://www.zhihu.com/api/v4/answers/1/voters", Some("{}"))
                .unwrap(),
            write_expr("DELETE", "https://www.zhihu.com/api/v4/collections/contents/answer/1", None)
                .unwrap(),
        ] {
            assert!(expr.contains("const tin"), "tin 必须声明: {expr}");
            assert!(expr.contains("tin,") || expr.contains("tin "), "tin 要被用上");
            assert!(!expr.contains("t_in"), "不能出现未声明的 t_in: {expr}");
            assert!(expr.contains("tout: Date.now()"), "tout 要就地取");
        }
    }
}
