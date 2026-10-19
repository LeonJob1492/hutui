use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::cdp::Cdp;
use super::{chrome, Cmd, Event};
use crate::config;
use crate::zhihu::parse;
use crate::zhihu::AnswerItem;

const HOME: &str = "https://www.zhihu.com";
const FEED_URL: &str = "https://www.zhihu.com/api/v3/feed/topstory/recommend?limit=10&desktop=true";
/// 单篇回答要的字段
const ANSWER_INCLUDE: &str = "content,excerpt,author,voteup_count,comment_count,question";
/// 回答列表要的字段
const LIST_INCLUDE: &str = "data[*].content,excerpt,author,voteup_count,comment_count,question";

pub fn run(rx: Receiver<Cmd>, tx: Sender<Event>) {
    let mut ctx = Ctx::default();
    while let Ok(cmd) = rx.recv() {
        let result = match cmd {
            Cmd::LoadFeed { want } => ctx.load_feed(&tx, want),
            Cmd::OpenAnswer { qid, aid } => ctx.open_answer(&tx, &qid, &aid),
        };
        if let Err(e) = result {
            let _ = tx.send(Event::Error(format!("{e:#}")));
        }
    }
}

#[derive(Default)]
struct Ctx {
    cdp: Option<Cdp>,
    session: Option<String>,
}

/// include 参数里的方括号星号要转义，否则页面 fetch 会当成通配
fn enc(include: &str) -> String {
    include
        .replace('[', "%5B")
        .replace(']', "%5D")
        .replace('*', "%2A")
}

impl Ctx {
    fn ensure(&mut self, tx: &Sender<Event>) -> Result<()> {
        if self.cdp.is_some() {
            return Ok(());
        }

        let _ = tx.send(Event::Status("启动浏览器…".into()));
        chrome::ensure(config::CDP_PORT)?;

        let _ = tx.send(Event::Status("连接调试端口…".into()));
        let mut cdp = Cdp::connect(config::CDP_PORT)?;

        // 直接建一个知乎页当同源上下文：fetch 必须在 zhihu.com 源下才带得上 cookie
        let created = cdp.call("Target.createTarget", json!({ "url": format!("{HOME}/") }), None)?;
        let target_id = created["targetId"]
            .as_str()
            .ok_or_else(|| anyhow!("createTarget 没返回 targetId"))?
            .to_string();
        let attached = cdp.call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| anyhow!("attachToTarget 没返回 sessionId"))?
            .to_string();
        cdp.call("Runtime.enable", json!({}), Some(&session))?;

        self.cdp = Some(cdp);
        self.session = Some(session);
        self.wait_origin(tx)?;
        Ok(())
    }

    /// 等页面真正落到 zhihu.com 域，否则 fetch 会被 CORS 挡掉
    fn wait_origin(&mut self, tx: &Sender<Event>) -> Result<()> {
        let _ = tx.send(Event::Status("等待知乎页面就绪…".into()));
        let session = self
            .session
            .clone()
            .ok_or_else(|| anyhow!("桥接尚未建立会话"))?;
        let deadline = Instant::now() + Duration::from_secs(25);
        while Instant::now() < deadline {
            let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;
            if let Ok(r) = cdp.call(
                "Runtime.evaluate",
                json!({ "expression": "location.origin", "returnByValue": true }),
                Some(&session),
            ) {
                if r["result"]["value"].as_str() == Some(HOME) {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        bail!("知乎页面在 25s 内没就绪，检查 profile 登录态")
    }

    fn fetch(&mut self, url: &str) -> Result<serde_json::Value> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| anyhow!("桥接尚未建立会话"))?;
        let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;
        cdp.fetch_json(&session, url)
    }

    /// 推荐流：首页一页 6 条，之后跟着 paging.next 翻
    fn load_feed(&mut self, tx: &Sender<Event>, want: usize) -> Result<()> {
        self.ensure(tx)?;
        let _ = tx.send(Event::Status("拉取推荐流…".into()));

        let mut acc: Vec<AnswerItem> = Vec::new();
        let mut url = FEED_URL.to_string();

        for _ in 0..config::FEED_MAX_PAGES {
            let v = self.fetch(&url)?;
            let items = parse::parse_feed_page(&v);
            if items.is_empty() {
                break;
            }
            append_unique(&mut acc, items);
            let _ = tx.send(Event::Feed(acc.clone()));
            if acc.len() >= want {
                break;
            }
            match parse::next_page_url(&v) {
                Some(next) => url = next,
                None => break,
            }
        }

        let _ = tx.send(Event::Status(format!("{} items", acc.len())));
        Ok(())
    }

    /// a：当前这条回答；b：该问题下的回答列表
    fn open_answer(&mut self, tx: &Sender<Event>, qid: &str, aid: &str) -> Result<()> {
        self.ensure(tx)?;

        // ---- a
        let _ = tx.send(Event::Status("拉取这条回答…".into()));
        let url = format!("{HOME}/api/v4/answers/{aid}?include={}", enc(ANSWER_INCLUDE));
        match self.fetch(&url) {
            Ok(v) => {
                if let Some(item) = parse::parse_answer_entity(&v) {
                    let _ = tx.send(Event::Answer(item));
                }
            }
            Err(e) => {
                // 单篇失败不影响列表，列表拿到也能读
                let _ = tx.send(Event::Status(format!("这条回答取不到：{e}")));
            }
        }

        // ---- b
        if qid.is_empty() {
            let _ = tx.send(Event::Status("这条没有所属问题".into()));
            return Ok(());
        }

        let _ = tx.send(Event::Status("拉取同问题回答…".into()));
        let mut acc: Vec<AnswerItem> = Vec::new();
        let mut offset = 0usize;
        let limit = config::ANSWERS_PAGE as usize;

        // 接口的 paging.totals 直接给出总数，翻到 is_end 或触顶就停
        let total = loop {
            let url = format!(
                "{HOME}/api/v4/questions/{qid}/answers?include={}&offset={offset}&limit={limit}&order=default&platform=desktop",
                enc(LIST_INCLUDE)
            );
            let v = self.fetch(&url)?;
            let (items, total, is_end) = parse::parse_answers_page(&v);
            if items.is_empty() {
                break total;
            }
            offset += items.len();
            append_unique(&mut acc, items);
            let _ = tx.send(Event::Answers {
                items: acc.clone(),
                total,
            });
            if is_end || acc.len() >= config::STREAM_MAX {
                break total;
            }
        };

        let _ = tx.send(Event::Status(format!("{total} 条回答")));
        Ok(())
    }
}

fn append_unique(acc: &mut Vec<AnswerItem>, items: Vec<AnswerItem>) {
    for it in items {
        if it.content.is_empty() {
            continue;
        }
        if !acc.iter().any(|x| x.id == it.id) {
            acc.push(it);
        }
    }
}
