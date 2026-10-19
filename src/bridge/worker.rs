use anyhow::{anyhow, bail, Result};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::cdp::Cdp;
use super::{chrome, Cmd, Event, Queued};
use crate::config;
use crate::trace;
use crate::zhihu::parse;
use crate::zhihu::{AnswerItem, Vote};

const HOME: &str = "https://www.zhihu.com";
const FEED_URL: &str = "https://www.zhihu.com/api/v3/feed/topstory/recommend?limit=10&desktop=true";
const ANSWER_INCLUDE: &str =
    "content,excerpt,author,voteup_count,comment_count,created_time,question,relationship.voting";
const LIST_INCLUDE: &str = "data[*].content,excerpt,author,voteup_count,comment_count,created_time,question,relationship.voting";

fn op_name(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::LoadFeed { .. } => "LoadFeed",
        Cmd::ResumeFeed { .. } => "ResumeFeed",
        Cmd::OpenAnswer { .. } => "OpenAnswer",
        Cmd::Vote { .. } => "Vote",
        Cmd::Favorite { .. } => "Favorite",
        Cmd::Comments { .. } => "Comments",
    }
}

/// 只有「换内容」的命令才值得打断正在跑的长任务。
fn supersedes(cmd: &Cmd) -> bool {
    // ResumeFeed 是续作，不该再打断别人
    matches!(cmd, Cmd::OpenAnswer { .. } | Cmd::LoadFeed { .. })
}

/// 收进 backlog。重复打开只保留最后一次，其余命令一条都不能丢。
fn push_cmd(backlog: &mut VecDeque<Queued>, job: Queued) {
    if matches!(job.cmd, Cmd::OpenAnswer { .. }) {
        backlog.retain(|q| !matches!(q.cmd, Cmd::OpenAnswer { .. }));
    }
    backlog.push_back(job);
}

/// 出错时用来只清对应那条在途标记
fn action_key(cmd: &Cmd) -> Option<String> {
    match cmd {
        Cmd::Vote { aid, .. } => Some(format!("{aid}:v")),
        Cmd::Favorite { aid, .. } => Some(format!("{aid}:f")),
        Cmd::Comments { .. } => Some("comments".into()),
        _ => None,
    }
}

pub fn run(rx: Receiver<Queued>, tx: Sender<Event>) {
    let mut ctx = Ctx::default();
    let mut backlog: VecDeque<Queued> = VecDeque::new();

    loop {
        // 先清积压，再去等新命令
        let job = match backlog.pop_front() {
            Some(job) => job,
            None => match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
        };

        trace::log(&format!(
            "cmd {} queued={}",
            op_name(&job.cmd),
            trace::ms(job.at.elapsed())
        ));

        let action = action_key(&job.cmd);
        let result = match job.cmd {
            Cmd::LoadFeed { want } => {
                // 用户主动刷新：丢掉旧的续作状态
                ctx.feed_resume = None;
                ctx.feed_acc.clear();
                ctx.load_feed(&tx, &rx, &mut backlog, want)
            }
            Cmd::ResumeFeed { want } => ctx.load_feed(&tx, &rx, &mut backlog, want),
            Cmd::OpenAnswer { qid, aid, req } => {
                ctx.open_answer(&tx, &rx, &mut backlog, &qid, &aid, req)
            }
            Cmd::Vote { aid, kind, vote } => ctx.vote(&tx, &aid, &kind, vote),
            Cmd::Favorite { aid, kind, on } => ctx.favorite(&tx, &aid, &kind, on),
            Cmd::Comments { aid, kind, offset } => ctx.comments(&tx, &aid, &kind, offset),
        };

        if let Err(e) = result {
            let _ = tx.send(Event::Error {
                msg: format!("{e:#}"),
                action,
            });
        }
        // 不管这条命令是自己失败还是就地执行的命令把连接搞挂了，都统一检查
        if ctx.is_dead() {
            ctx.reset();
        }
    }
}

#[derive(Default)]
struct Ctx {
    cdp: Option<Cdp>,
    session: Option<String>,
    /// 默认收藏夹 id，懒加载后缓存
    collection: Option<String>,
    /// 被打断时存下的「下一页 URL」，续作从这里接着拉
    feed_resume: Option<String>,
    /// 被打断时已经拉到的条目，续作要在它之上继续累加
    feed_acc: Vec<AnswerItem>,
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

    /// 把队列里的命令收进 backlog；顺手把「不打断加载」的命令就地执行掉，
    /// 这样按 z 不用等三页翻完。返回 true 表示该收工让路了。
    fn service(
        &mut self,
        tx: &Sender<Event>,
        rx: &Receiver<Queued>,
        backlog: &mut VecDeque<Queued>,
    ) -> bool {
        while let Ok(job) = rx.try_recv() {
            push_cmd(backlog, job);
        }

        let mut i = 0;
        while i < backlog.len() {
            if supersedes(&backlog[i].cmd) {
                i += 1;
                continue;
            }
            let job = backlog.remove(i).expect("下标在范围内");
            trace::log(&format!(
                "cmd {} queued={} inline",
                op_name(&job.cmd),
                trace::ms(job.at.elapsed())
            ));
            let action = action_key(&job.cmd);
            if let Err(e) = self.run_quick(job.cmd, tx) {
                let _ = tx.send(Event::Error {
                    msg: format!("{e:#}"),
                    action,
                });
            }
        }

        backlog.iter().any(|q| supersedes(&q.cmd))
    }

    fn run_quick(&mut self, cmd: Cmd, tx: &Sender<Event>) -> Result<()> {
        match cmd {
            Cmd::Vote { aid, kind, vote } => self.vote(tx, &aid, &kind, vote),
            Cmd::Favorite { aid, kind, on } => self.favorite(tx, &aid, &kind, on),
            Cmd::Comments { aid, kind, offset } => self.comments(tx, &aid, &kind, offset),
            // 长任务不该走到这里
            _ => Ok(()),
        }
    }

    fn ensure(&mut self, tx: &Sender<Event>) -> Result<()> {
        if self.cdp.is_some() {
            return Ok(());
        }

        let _ = tx.send(Event::Status("booting browser…".into()));
        chrome::ensure(config::CDP_PORT)?;

        let _ = tx.send(Event::Status("attaching debugger…".into()));
        let t0 = Instant::now();
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

        let reused = existing.is_some();
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
        trace::log(&format!(
            "attach reused={reused} total={}",
            trace::ms(t0.elapsed())
        ));
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
        rx: &Receiver<Queued>,
        backlog: &mut VecDeque<Queued>,
        want: usize,
    ) -> Result<()> {
        self.ensure(tx)?;

        let (mut acc, mut url) = match self.feed_resume.take() {
            Some(next) => (std::mem::take(&mut self.feed_acc), next),
            None => (Vec::new(), FEED_URL.to_string()),
        };
        let _ = tx.send(Event::Status(if acc.is_empty() {
            "fetching feed…".into()
        } else {
            "resuming feed…".into()
        }));

        for _ in 0..config::FEED_MAX_PAGES {
            let v = self.fetch(&url)?;
            let items = parse::parse_feed_page(&v);
            if items.is_empty() {
                break;
            }
            append_unique(&mut acc, items);
            let _ = tx.send(Event::Feed(acc.clone()));
            trace::log(&format!("feed n={} want={want}", acc.len()));
            if acc.len() >= want {
                break;
            }
            let Some(next) = parse::next_page_url(&v) else {
                break;
            };
            if self.service(tx, rx, backlog) {
                // 让路：把「从 next 接着拉」放回队列尾部，等前面的事做完再继续
                self.feed_resume = Some(next);
                self.feed_acc = acc;
                backlog.push_back(Queued {
                    cmd: Cmd::ResumeFeed { want },
                    at: Instant::now(),
                });
                trace::log("feed paused, resume queued");
                return Ok(());
            }
            url = next;
        }

        self.feed_resume = None;
        self.feed_acc.clear();
        let _ = tx.send(Event::Status(format!("{} items", acc.len())));
        Ok(())
    }

    fn open_answer(
        &mut self,
        tx: &Sender<Event>,
        rx: &Receiver<Queued>,
        backlog: &mut VecDeque<Queued>,
        qid: &str,
        aid: &str,
        req: u64,
    ) -> Result<()> {
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
            return Ok(());
        }

        let _ = tx.send(Event::Status("fetching siblings…".into()));
        let mut acc: Vec<AnswerItem> = Vec::new();
        let mut offset = 0usize;
        let limit = config::ANSWERS_PAGE;

        loop {
            // 队列里如果有「换内容」的命令就收工让路；其余命令就地执行掉
            if self.service(tx, rx, backlog) {
                return Ok(());
            }

            let url = format!(
                "{HOME}/api/v4/questions/{qid}/answers?include={}&offset={offset}&limit={limit}&order=default&platform=desktop",
                enc(LIST_INCLUDE)
            );
            let v = self.fetch(&url)?;
            let (items, total, is_end) = parse::parse_answers_page(&v);
            if items.is_empty() {
                break;
            }
            offset += items.len();
            append_unique(&mut acc, items);
            let _ = tx.send(Event::Answers {
                items: acc.clone(),
                total,
                req,
            });
            trace::log(&format!(
                "answers req={req} n={} total={total} offset={offset}",
                acc.len()
            ));
            if is_end || acc.len() >= config::STREAM_MAX {
                break;
            }
        }

        let _ = tx.send(Event::Status(format!("{} answers", acc.len())));
        Ok(())
    }

    fn comments(&mut self, tx: &Sender<Event>, aid: &str, kind: &str, offset: usize) -> Result<()> {
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
        Ok(())
    }

    // ---------------------------------------------------------------- 写

    fn vote(&mut self, tx: &Sender<Event>, aid: &str, kind: &str, vote: Vote) -> Result<()> {
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
        Ok(())
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

    fn favorite(&mut self, tx: &Sender<Event>, aid: &str, kind: &str, on: bool) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(cmd: Cmd) -> Queued {
        Queued {
            cmd,
            at: Instant::now(),
        }
    }

    fn open(aid: &str, req: u64) -> Cmd {
        Cmd::OpenAnswer {
            qid: "q".into(),
            aid: aid.into(),
            req,
        }
    }

    fn vote(aid: &str) -> Cmd {
        Cmd::Vote {
            aid: aid.into(),
            kind: "answer".into(),
            vote: Vote::Up,
        }
    }

    #[test]
    fn only_content_switches_supersede() {
        assert!(supersedes(&open("a", 1)));
        assert!(supersedes(&Cmd::LoadFeed { want: 10 }));
        assert!(!supersedes(&vote("a")));
        assert!(!supersedes(&Cmd::Comments {
            aid: "a".into(),
            kind: "answer".into(),
            offset: 0
        }));
    }

    /// 关键：Vote / Comments 一条都不能丢，只合并重复的 OpenAnswer
    #[test]
    fn backlog_never_drops_actions() {
        let mut b: VecDeque<Queued> = VecDeque::new();
        push_cmd(&mut b, queued(vote("a")));
        push_cmd(&mut b, queued(open("x", 1)));
        push_cmd(
            &mut b,
            queued(Cmd::Comments {
                aid: "a".into(),
                kind: "answer".into(),
                offset: 0,
            }),
        );
        push_cmd(&mut b, queued(open("y", 2)));

        assert_eq!(b.len(), 3, "两次 OpenAnswer 合并成一条，其余保留");
        assert!(matches!(b[0].cmd, Cmd::Vote { .. }));
        assert!(matches!(b[1].cmd, Cmd::Comments { .. }));
        assert!(
            matches!(&b[2].cmd, Cmd::OpenAnswer { aid, req, .. } if aid == "y" && *req == 2)
        );
    }

    /// 推荐流让路后，续作必须排在「打开回答」后面，且不能被合并掉
    #[test]
    fn resume_feed_queues_after_open() {
        let mut b: VecDeque<Queued> = VecDeque::new();
        push_cmd(&mut b, queued(open("a", 1)));
        push_cmd(&mut b, queued(Cmd::ResumeFeed { want: 42 }));

        assert_eq!(b.len(), 2, "续作不能被合并掉");
        assert!(
            matches!(b[0].cmd, Cmd::OpenAnswer { .. }),
            "续作要排在打开之后，等它先完成"
        );
        assert!(matches!(b[1].cmd, Cmd::ResumeFeed { .. }));
        assert!(
            !supersedes(&Cmd::ResumeFeed { want: 42 }),
            "续作本身不该再打断别人"
        );
    }

    #[test]
    fn action_keys_map_to_pending_marks() {
        assert_eq!(action_key(&vote("a")).as_deref(), Some("a:v"));
        assert_eq!(
            action_key(&Cmd::Favorite {
                aid: "a".into(),
                kind: "answer".into(),
                on: true
            })
            .as_deref(),
            Some("a:f")
        );
        assert_eq!(
            action_key(&Cmd::Comments {
                aid: "a".into(),
                kind: "answer".into(),
                offset: 0
            })
            .as_deref(),
            Some("comments")
        );
        assert_eq!(action_key(&open("a", 1)), None);
    }
}
