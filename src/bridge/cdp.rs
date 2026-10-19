//! 极简 CDP 客户端：WebSocket + JSON-RPC，够用就好。
//!
//! 只用 `Network` / `Page` / `Input` / `Target` / `Runtime` 几个域。

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub struct Capture {
    pub body: Value,
}

pub struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    responses: HashMap<u64, Value>,
    events: Vec<Value>,
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
                ) =>
            {
                // 端点还没吐完整 body，继续等
            }
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
            events: Vec::new(),
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
        } else if v.get("method").is_some() {
            self.events.push(v);
        }
    }

    /// 读一段时间，把响应与事件都收进队列
    pub fn pump(&mut self, dur: Duration) -> Result<()> {
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
        let deadline = Instant::now() + Duration::from_secs(25);
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
            self.pump(Duration::from_millis(50))?;
        }
    }

    /// 等待 `dur` 长的时间窗口，收集所有命中 `url_contains` 的 JSON 响应体。
    ///
    /// 我们不发请求 —— 只是让页面自己跑，然后把它自己拉的 JSON 搬回来。
    /// 一旦有命中，再给 `grace` 的宽限就提前返回，避免傻等满窗口。
    pub fn capture_json(
        &mut self,
        session: &str,
        url_contains: &[&str],
        dur: Duration,
    ) -> Result<Vec<Capture>> {
        const GRACE: Duration = Duration::from_millis(700);
        let deadline = Instant::now() + dur;
        let mut first_hit: Option<Instant> = None;
        let mut seen = 0usize;

        loop {
            self.pump(Duration::from_millis(120))?;

            let n = self.count_hits(session, url_contains);
            if n > seen {
                seen = n;
                first_hit.get_or_insert_with(Instant::now);
            }
            if let Some(t) = first_hit {
                if t.elapsed() >= GRACE {
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
        }

        let events = std::mem::take(&mut self.events);

        let mut url_by_req: HashMap<String, String> = HashMap::new();
        let mut finished: Vec<String> = Vec::new();

        for e in &events {
            if e.get("sessionId").and_then(Value::as_str) != Some(session) {
                continue;
            }
            let method = e.get("method").and_then(Value::as_str).unwrap_or("");
            let p = &e["params"];
            match method {
                "Network.responseReceived" => {
                    let url = p["response"]["url"].as_str().unwrap_or("");
                    if url_contains.iter().any(|needle| url.contains(needle)) {
                        let rid = p["requestId"].as_str().unwrap_or("").to_string();
                        if !rid.is_empty() {
                            url_by_req.insert(rid, url.to_string());
                        }
                    }
                }
                "Network.loadingFinished" => {
                    let rid = p["requestId"].as_str().unwrap_or("").to_string();
                    if url_by_req.contains_key(&rid) {
                        finished.push(rid);
                    }
                }
                _ => {}
            }
        }

        let mut out = Vec::new();
        for rid in finished {
            let Ok(res) = self.call(
                "Network.getResponseBody",
                json!({ "requestId": rid }),
                Some(session),
            ) else {
                continue;
            };
            if res["base64Encoded"].as_bool().unwrap_or(false) {
                continue;
            }
            let body = res["body"].as_str().unwrap_or("");
            if let Ok(json) = serde_json::from_str::<Value>(body) {
                out.push(Capture { body: json });
            }
        }
        Ok(out)
    }

    fn count_hits(&self, session: &str, url_contains: &[&str]) -> usize {
        self.events
            .iter()
            .filter(|e| e.get("sessionId").and_then(Value::as_str) == Some(session))
            .filter(|e| {
                e.get("method").and_then(Value::as_str) == Some("Network.responseReceived")
            })
            .filter(|e| {
                let url = e["params"]["response"]["url"].as_str().unwrap_or("");
                url_contains.iter().any(|n| url.contains(n))
            })
            .count()
    }

    /// 在页面里求值（只读用途）
    pub fn evaluate(&mut self, session: &str, expr: &str) -> Result<Value> {
        self.call(
            "Runtime.evaluate",
            json!({ "expression": expr, "returnByValue": true }),
            Some(session),
        )
    }

    pub fn scroll(&mut self, session: &str, dy: i64) -> Result<()> {
        self.call(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": 520, "y": 380,
                "deltaX": 0, "deltaY": dy
            }),
            Some(session),
        )?;
        Ok(())
    }

    pub fn navigate(&mut self, session: &str, url: &str) -> Result<()> {
        self.call("Page.navigate", json!({ "url": url }), Some(session))?;
        Ok(())
    }
}
