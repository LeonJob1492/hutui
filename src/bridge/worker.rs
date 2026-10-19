use anyhow::{anyhow, Result};
use serde_json::json;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use super::cdp::{Capture, Cdp};
use super::{chrome, Cmd, Event};
use crate::config;
use crate::zhihu::parse;
use crate::zhihu::AnswerItem;

const HOME: &str = "https://www.zhihu.com";
/// 注意是 v3，不是 v4：只过滤 /api/v4/ 会把它整个漏掉
const FEED_API: &str = "/api/v3/feed/topstory/recommend";

pub fn run(rx: Receiver<Cmd>, tx: Sender<Event>) {
    let mut ctx = Ctx::default();
    while let Ok(cmd) = rx.recv() {
        let result = match cmd {
            Cmd::LoadFeed { want } => ctx.load_feed(&tx, want),
            Cmd::LoadAnswers { qid } => ctx.load_answers(&tx, &qid),
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

impl Ctx {
    fn ensure(&mut self, tx: &Sender<Event>) -> Result<()> {
        if self.cdp.is_some() {
            return Ok(());
        }

        let _ = tx.send(Event::Status("启动浏览器…".into()));
        chrome::ensure(config::CDP_PORT)?;

        let _ = tx.send(Event::Status("连接调试端口…".into()));
        let mut cdp = Cdp::connect(config::CDP_PORT)?;

        let created = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None)?;
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

        cdp.call(
            "Network.enable",
            json!({ "maxTotalBufferSize": 200_000_000u64, "maxResourceBufferSize": 60_000_000u64 }),
            Some(&session),
        )?;
        cdp.call("Page.enable", json!({}), Some(&session))?;
        cdp.call("Runtime.enable", json!({}), Some(&session))?;

        self.cdp = Some(cdp);
        self.session = Some(session);
        Ok(())
    }

    fn session(&self) -> Result<&str> {
        self.session
            .as_deref()
            .ok_or_else(|| anyhow!("桥接尚未建立会话"))
    }

    fn load_feed(&mut self, tx: &Sender<Event>, want: usize) -> Result<()> {
        self.ensure(tx)?;
        let session = self.session()?.to_string();
        let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;

        let _ = tx.send(Event::Status("打开推荐流…".into()));
        cdp.navigate(&session, &format!("{HOME}/"))?;

        let mut acc: Vec<AnswerItem> = Vec::new();

        let caps = cdp.capture_json(
            &session,
            &[FEED_API],
            Duration::from_millis(config::FEED_TIMEOUT_MS),
        )?;
        absorb_feed(&caps, &mut acc);
        let _ = tx.send(Event::Feed(acc.clone()));

        for _ in 0..config::FEED_PAGE_SCROLLS {
            if acc.len() >= want {
                break;
            }
            cdp.scroll(&session, 2200)?;
            std::thread::sleep(Duration::from_millis(900));
            let caps = cdp.capture_json(
                &session,
                &[FEED_API],
                Duration::from_millis(config::PAGE_TIMEOUT_MS),
            )?;
            absorb_feed(&caps, &mut acc);
            let _ = tx.send(Event::Feed(acc.clone()));
        }

        let _ = tx.send(Event::Status(format!("{} items", acc.len())));
        Ok(())
    }

    fn load_answers(&mut self, tx: &Sender<Event>, qid: &str) -> Result<()> {
        self.ensure(tx)?;
        let session = self.session()?.to_string();
        let cdp = self.cdp.as_mut().ok_or_else(|| anyhow!("桥接未连接"))?;

        let api = format!("/api/v4/questions/{qid}/feeds");
        let _ = tx.send(Event::Status("拉同问题回答…".into()));
        // 问题页滚动会自动翻页；回答页要点按钮，所以走问题页
        cdp.navigate(&session, &format!("{HOME}/question/{qid}"))?;

        let mut acc: Vec<AnswerItem> = Vec::new();
        let mut is_end = false;

        for _ in 0..=config::ANSWER_PAGE_SCROLLS {
            let caps = cdp.capture_json(
                &session,
                &[api.as_str()],
                Duration::from_millis(if acc.is_empty() {
                    config::FEED_TIMEOUT_MS
                } else {
                    config::PAGE_TIMEOUT_MS
                }),
            )?;
            for c in &caps {
                let (items, end) = parse::parse_question_feeds(&c.body);
                is_end = end;
                append_unique(&mut acc, items);
            }
            let _ = tx.send(Event::Answers {
                items: acc.clone(),
                is_end,
            });
            if is_end || acc.len() >= config::STREAM_MAX {
                break;
            }
            cdp.evaluate(
                &session,
                "window.scrollTo(0, document.documentElement.scrollHeight)",
            )?;
            std::thread::sleep(Duration::from_millis(1200));
        }

        Ok(())
    }
}

fn absorb_feed(caps: &[Capture], acc: &mut Vec<AnswerItem>) {
    for c in caps {
        append_unique(acc, parse::parse_feed_page(&c.body));
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
