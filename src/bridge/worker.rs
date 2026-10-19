use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::cdp::Cdp;
use super::{chrome, Cmd, Event};
use crate::config;
use crate::zhihu::parse;
use crate::zhihu::{AnswerItem, Vote};

const HOME: &str = "https://www.zhihu.com";
const FEED_URL: &str = "https://www.zhihu.com/api/v3/feed/topstory/recommend?limit=10&desktop=true";
const ANSWER_INCLUDE: &str =
    "content,excerpt,author,voteup_count,comment_count,created_time,question,relationship.voting";
const LIST_INCLUDE: &str = "data[*].content,excerpt,author,voteup_count,comment_count,created_time,question,relationship.voting";

/// 长任务在翻页间隙看一眼有没有新命令，有就提前收工，把新命令交回主循环。
fn take_pending(rx: &Receiver<Cmd>) -> Option<Cmd> {
    let mut last = None;
    while let Ok(cmd) = rx.try_recv() {
        last = Some(cmd);
    }
    last
}

pub fn run(rx: Receiver<Cmd>, tx: Sender<Event>) {
    let mut ctx = Ctx::default();
    let mut pending: Option<Cmd> = None;

    loop {
        let cmd = match pending.take() {
            Some(cmd) => cmd,
            None => match rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => break,
            },
        };

        let result = match cmd {
            Cmd::LoadFeed { want } => ctx.load_feed(&tx, &rx, want),
            Cmd::OpenAnswer { qid, aid, req } => ctx.open_answer(&tx, &rx, &qid, &aid, req),
            Cmd::Vote { aid, kind, vote } => ctx.vote(&tx, &aid, &kind, vote),
            Cmd::Favorite { aid, kind, on } => ctx.favorite(&tx, &aid, &kind, on),
            Cmd::Comments { aid, kind, offset } => ctx.comments(&tx, &aid, &kind, offset),
        };

        match result {
            Ok(Some(next)) => pending = Some(next),
            Ok(None) => {}
            Err(e) => {
                let _ = tx.send(Event::Error(format!("{e:#}")));
                // 连接或目标已经死掉：清空会话，下一条命令会自动重连
                if ctx.is_dead() {
                    ctx.reset();
                }
            }
        }
    }
}

#[derive(Default)]
struct Ctx {
    cdp: Option<Cdp>,
    session: Option<String>,
    /// 默认收藏夹 id，懒加载后缓存
    collection: Option<String>,
}

/// include 参数里的方括号星号要转义
fn enc(include: &str) -> String {
    include
        .replace('[', "%5B")
        .replace(']', "%5D")
        .replace('*', "%2A")
}

/// 内容类型 → 接口路径段
fn seg(kind: &str) -> &'static str {
    match kind {
        "article" => "articles",
        "pin" => "pins",
        _ => "answers",
    }
}

/// 内容类型 → 收藏用的 content_type
fn content_type(kind: &str) -> &'static str {
    match kind {
        "article" => "article",
        "pin" => "pin",
        _ => "answer",
    }
}

impl Ctx {
    fn is_dead(&self) -> bool {
        self.cdp.as_ref().map_or(false, Cdp::is_dead)
    }

    fn reset(&mut self) {
        self.cdp = None;
        self.session = None;
    }

    fn ensure(&mut self, tx: &Sender<Event>) -> Result<()> {
        if self.cdp.is_some() {
            return Ok(());
        }

        let _ = tx.send(Event::Status("booting browser…".into()));
        chrome::ensure(config::CDP_PORT)?;

        let _ = tx.send(Event::Status("attaching debugger…".into()));
        let mut cdp = Cdp::connect(config::CDP_PORT)?;

        // 已经有知乎标签页就直接复用，避免每次启动都堆一个
        let existing = cdp
            .call("Target.getTargets", json!({}), None)
            .ok()
            .and_then(|v| {
                v["targetInfos"].as_array().and_then(|arr| {
                    arr.iter()
                        .find(|t| {
                            t["type"].as_str() == Some("page")
                                && t["url"].as_str().map_or(false, |u| u.starts_with(HOME))
                        })
                        .and_then(|t| t["targetId"].as_str().map(str::to_string))
                })
            });

        let target_id = match existing {
            Some(id) => id,
            None => {
                let created =
                    cdp.call("Target.createTarget", json!({ "url": format!("{HOME}/") }), None)?;
                created["targetId"]
                    .as_str()
                    .ok_or_else(|| anyhow!("createTarget 没返回 targetId"))?
                    .to_string()
            }
        };

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

    fn wait_origin(&mut self, tx: &Sender<Event>) -> Result<()> {
        let _ = tx.send(Event::Status("waiting for session…".into()));
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
        bail!("浏览器 25s 内没就绪，检查 profile 登录态")
    }

    fn fetch(&mut self, url: &str) -> Result<serde_json::Value> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| anyhow!("桥接尚未建立会话"))?;
        let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;
        cdp.fetch_json(&session, url)
    }

    fn write(&mut self, method: &str, url: &str, body: Option<&str>) -> Result<serde_json::Value> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| anyhow!("桥接尚未建立会话"))?;
        let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;
        cdp.write_json(&session, method, url, body)
    }

    // ---------------------------------------------------------------- 读

    fn load_feed(
        &mut self,
        tx: &Sender<Event>,
        rx: &Receiver<Cmd>,
        want: usize,
    ) -> Result<Option<Cmd>> {
        self.ensure(tx)?;
        let _ = tx.send(Event::Status("fetching feed…".into()));

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
            if let Some(next) = take_pending(rx) {
                return Ok(Some(next));
            }
            match parse::next_page_url(&v) {
                Some(next) => url = next,
                None => break,
            }
        }

        let _ = tx.send(Event::Status(format!("{} items", acc.len())));
        Ok(None)
    }

    fn open_answer(
        &mut self,
        tx: &Sender<Event>,
        rx: &Receiver<Cmd>,
        qid: &str,
        aid: &str,
        req: u64,
    ) -> Result<Option<Cmd>> {
        self.ensure(tx)?;

        let _ = tx.send(Event::Status("fetching answer…".into()));
        let url = format!("{HOME}/api/v4/answers/{aid}?include={}", enc(ANSWER_INCLUDE));
        match self.fetch(&url) {
            Ok(v) => {
                if let Some(item) = parse::parse_answer_entity(&v) {
                    let _ = tx.send(Event::Answer { item, req });
                }
            }
            Err(e) => {
                let _ = tx.send(Event::Status(format!("answer unavailable: {e}")));
            }
        }

        if qid.is_empty() {
            let _ = tx.send(Event::Status("no parent thread".into()));
            return Ok(None);
        }

        let _ = tx.send(Event::Status("fetching siblings…".into()));
        let mut acc: Vec<AnswerItem> = Vec::new();
        let mut offset = 0usize;
        let limit = config::ANSWERS_PAGE;

        let total = loop {
            // 用户在翻页间隙又打开了别的东西：立刻让路
            if let Some(next) = take_pending(rx) {
                return Ok(Some(next));
            }

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
                req,
            });
            if is_end || acc.len() >= config::STREAM_MAX {
                break total;
            }
        };

        let _ = tx.send(Event::Status(format!("{total} answers")));
        Ok(None)
    }

    fn comments(
        &mut self,
        tx: &Sender<Event>,
        aid: &str,
        kind: &str,
        offset: usize,
    ) -> Result<Option<Cmd>> {
        self.ensure(tx)?;
        let url = format!(
            "{HOME}/api/v4/{0}/{aid}/comments?limit={1}&offset={offset}&order=normal",
            seg(kind),
            config::COMMENTS_PAGE
        );
        let v = self.fetch(&url)?;
        let (items, total) = parse::parse_comments(&v);
        let next = offset + items.len();
        let _ = tx.send(Event::Comments {
            aid: aid.to_string(),
            items,
            total,
            next_offset: next,
        });
        Ok(None)
    }

    // ---------------------------------------------------------------- 写

    fn vote(
        &mut self,
        tx: &Sender<Event>,
        aid: &str,
        kind: &str,
        vote: Vote,
    ) -> Result<Option<Cmd>> {
        self.ensure(tx)?;
        let _ = tx.send(Event::Status("sending…".into()));

        let url = format!("{HOME}/api/v4/{}/{aid}/voters", seg(kind));
        let body = format!("{{\"type\":\"{}\"}}", vote.as_api());
        let v = self.write("POST", &url, Some(&body))?;

        let vote = Vote::from_api(v["reaction_value"].as_str().unwrap_or(""));
        let count = v["reaction_count"].as_i64().unwrap_or(0);
        let _ = tx.send(Event::Voted {
            aid: aid.to_string(),
            vote,
            count,
        });
        Ok(None)
    }

    fn default_collection(&mut self) -> Result<String> {
        if let Some(c) = &self.collection {
            return Ok(c.clone());
        }
        let me = self.fetch(&format!("{HOME}/api/v4/me"))?;
        let token = me["url_token"].as_str().unwrap_or_default().to_string();
        if token.is_empty() {
            bail!("拿不到自己的 url_token");
        }
        let list = self.fetch(&format!(
            "{HOME}/api/v4/people/{token}/collections?offset=0&limit=20"
        ))?;
        let id = parse::parse_default_collection(&list).ok_or_else(|| anyhow!("找不到默认收藏夹"))?;
        self.collection = Some(id.clone());
        Ok(id)
    }

    fn favorite(
        &mut self,
        tx: &Sender<Event>,
        aid: &str,
        kind: &str,
        on: bool,
    ) -> Result<Option<Cmd>> {
        self.ensure(tx)?;
        let _ = tx.send(Event::Status(if on {
            "saving…".into()
        } else {
            "removing…".into()
        }));

        if on {
            let cid = self.default_collection()?;
            // 这个端点的参数走 query，body 必须为空
            let url = format!(
                "{HOME}/api/v4/collections/{cid}/contents?content_id={aid}&content_type={}",
                content_type(kind)
            );
            self.write("POST", &url, None)?;
        } else {
            let url = format!(
                "{HOME}/api/v4/collections/contents/{}/{aid}",
                content_type(kind)
            );
            self.write("DELETE", &url, None)?;
        }

        let _ = tx.send(Event::Faved {
            aid: aid.to_string(),
            on,
        });
        Ok(None)
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
