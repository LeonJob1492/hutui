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
        })
    }

    fn push(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.ws
            .send(Message::Text(msg.to_string()))
            .context("发送 CDP 消息失败")?;
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
                Ok(Message::Close(_)) => bail!("CDP 连接被对端关闭"),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(tungstenite::Error::ConnectionClosed) => bail!("CDP 连接已断开"),
                Err(e) => return Err(e.into()),
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
                    bail!("CDP {method} 出错: {err}");
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            if Instant::now() > deadline {
                bail!("CDP {method} 超时");
            }
            self.pump(Duration::from_millis(30))?;
        }
    }

    /// 在页面上下文里 `fetch(url)` 并把响应体当 JSON 返回。
    ///
    /// 用的是页面自己的同源上下文和登录 cookie —— 不做任何签名伪造，
    /// 这些接口本身也不校验签名（实测）。
    pub fn fetch_json(&mut self, session: &str, url: &str) -> Result<Value> {
        let quoted = serde_json::to_string(url)?;
        let expr = format!(
            "(async () => {{ \
               try {{ \
                 const r = await fetch({quoted}, {{ credentials: 'include' }}); \
                 const t = await r.text(); \
                 return {{ ok: r.ok, status: r.status, text: t }}; \
               }} catch (e) {{ return {{ ok: false, status: 0, text: String(e) }}; }} \
             }})()"
        );

        let res = self.call(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true, "awaitPromise": true }),
            Some(session),
        )?;

        let val = &res["result"]["value"];
        if val.is_null() {
            let detail = res
                .get("exceptionDetails")
                .map(|d| d.to_string())
                .unwrap_or_else(|| "无返回值".into());
            bail!("页面求值失败: {detail}");
        }

        let status = val["status"].as_i64().unwrap_or(0);
        let text = val["text"].as_str().unwrap_or("");
        if status != 200 {
            let head: String = text.chars().take(160).collect();
            bail!("{} → HTTP {status}: {head}", brief(url));
        }
        serde_json::from_str(text).with_context(|| format!("{} 返回的不是 JSON", brief(url)))
    }
}
