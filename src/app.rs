use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::bridge::{Cmd, Event as BridgeEvent, Tx};
use crate::config;
use crate::state::ReadLog;
use crate::ui;
use crate::zhihu::{AnswerItem, AnswerMeta, Comment, Vote};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Reading,
    Comments,
    Panic,
}

/// 评论区状态
pub struct CommentsState {
    pub aid: String,
    pub items: Vec<Comment>,
    pub lines: Vec<Line<'static>>,
    pub total: usize,
    pub next_offset: usize,
    pub scroll: usize,
    pub loading: bool,
}

impl Default for CommentsState {
    fn default() -> Self {
        Self {
            aid: String::new(),
            items: Vec::new(),
            lines: Vec::new(),
            total: 0,
            next_offset: 0,
            scroll: 0,
            loading: false,
        }
    }
}

pub struct App {
    pub view: View,
    /// 从 panic 恢复时回到哪个视图
    pub resume: View,

    // ---- 列表
    pub feed: Vec<AnswerItem>,
    pub cursor: usize,
    pub list_offset: usize,
    pub list_above: usize,
    pub list_below: usize,
    /// 看过的条目
    pub read: ReadLog,

    // ---- 阅读：一条一屏，↑↓ 换条
    pub stream: Vec<AnswerItem>,
    pub index: usize,
    pub total: usize,
    /// 吸顶信息条（分隔行 / 标题 / 作者），不参与滚动
    pub header: Vec<Line<'static>>,
    /// 可滚动正文
    pub lines: Vec<Line<'static>>,
    pub scroll: usize,

    /// 每条内容上我的表态（赞/收藏）
    pub meta: HashMap<String, AnswerMeta>,
    pub comments: CommentsState,

    // ---- 布局
    pub built_width: u16,
    pub body_h: u16,

    pub status: String,
    pub quit: bool,
    /// 需要重绘（只在有变化时才重绘，避免闲时空转）
    pub dirty: bool,
    /// 打开序号：旧序号的后台事件直接丢弃
    req: u64,
    /// 进行中的写操作，key 形如 "aid:v" / "aid:f"，防止连按重复提交
    pending: std::collections::HashSet<String>,
    last_esc: Option<Instant>,
    tx: Tx,
}

impl App {
    pub fn new(tx: Tx) -> Self {
        Self {
            view: View::List,
            resume: View::List,
            feed: Vec::new(),
            cursor: 0,
            list_offset: 0,
            list_above: 0,
            list_below: 0,
            read: ReadLog::load(),
            stream: Vec::new(),
            index: 0,
            total: 0,
            header: Vec::new(),
            lines: Vec::new(),
            scroll: 0,
            meta: HashMap::new(),
            comments: CommentsState::default(),
            built_width: 0,
            body_h: 0,
            status: "connecting…".into(),
            quit: false,
            dirty: true,
            req: 0,
            pending: std::collections::HashSet::new(),
            last_esc: None,
            tx,
        }
    }

    // ------------------------------------------------------------- 按键

    pub fn on_key(&mut self, k: KeyEvent) {
        self.dirty = true;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);

        if ctrl && matches!(k.code, KeyCode::Char('c')) {
            self.quit = true;
            return;
        }
        if ctrl && matches!(k.code, KeyCode::Char('h')) {
            self.toggle_panic();
            return;
        }
        // macOS 上 Ctrl-H 常被终端编码成 Ctrl-Backspace，两条都收
        if ctrl && matches!(k.code, KeyCode::Backspace) {
            self.toggle_panic();
            return;
        }

        if k.code == KeyCode::Esc {
            let now = Instant::now();
            let doubled = self
                .last_esc
                .map(|t| now.duration_since(t) < Duration::from_millis(520))
                .unwrap_or(false);
            if doubled {
                self.last_esc = None;
                self.toggle_panic();
            } else {
                self.last_esc = Some(now);
            }
            return;
        }
        self.last_esc = None;

        match self.view {
            // 应急画面下不响应任何普通键：手一抖按到空格/q 不应该把正文请回来
            View::Panic => {}
            View::List => self.on_key_list(k),
            View::Reading => self.on_key_reading(k),
            View::Comments => self.on_key_comments(k),
        }
    }

    fn on_key_list(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.feed.len() {
                    self.cursor += 1;
                }
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.feed.len().saturating_sub(1),
            KeyCode::Right | KeyCode::Enter => self.open_reading(),
            KeyCode::Char('r') => {
                self.status = "refreshing…".into();
                let _ = self.tx.send(Cmd::LoadFeed {
                    want: config::FEED_TARGET,
                });
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn on_key_reading(&mut self, k: KeyEvent) {
        match k.code {
            // ↑↓ 先在本条内翻屏，翻到边界才换条
            KeyCode::Up | KeyCode::Char('k') => self.page_up_or_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.page_down_or_next(),
            // n/p 只换条不翻屏
            KeyCode::Char('n') => self.step_answer(1),
            KeyCode::Char('p') => self.step_answer(-1),
            KeyCode::Char('z') => self.toggle_vote(Vote::Up),
            KeyCode::Char('v') => self.toggle_vote(Vote::Down),
            KeyCode::Char('c') => self.toggle_fav(),
            KeyCode::Char('x') => self.open_comments(),
            // PgUp/PgDn 只翻屏，不换条
            KeyCode::PageUp => self.page_only(false),
            KeyCode::PageDown => self.page_only(true),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),
            KeyCode::Left | KeyCode::Backspace => self.back_to_list(),
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn on_key_comments(&mut self, k: KeyEvent) {
        let page = (self.body_h.saturating_sub(1) as usize).max(1);
        let max = self
            .comments
            .lines
            .len()
            .saturating_sub(self.body_h as usize);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.comments.scroll = self.comments.scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.comments.scroll = (self.comments.scroll + 1).min(max)
            }
            KeyCode::PageUp => self.comments.scroll = self.comments.scroll.saturating_sub(page),
            KeyCode::PageDown => self.comments.scroll = (self.comments.scroll + page).min(max),
            KeyCode::Home => self.comments.scroll = 0,
            KeyCode::End => self.comments.scroll = max,
            KeyCode::Enter => self.load_more_comments(),
            KeyCode::Char('x') | KeyCode::Left | KeyCode::Backspace => {
                self.view = View::Reading;
                self.resume = View::Reading;
                self.status.clear();
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    // ------------------------------------------------------------- 动作

    fn toggle_panic(&mut self) {
        if self.view == View::Panic {
            self.view = self.resume;
            self.rebuild();
        } else {
            self.resume = self.view;
            self.view = View::Panic;
        }
    }

    fn current(&self) -> Option<&AnswerItem> {
        self.stream.get(self.index)
    }

    /// 把接口给的「我是否已赞」种进本地表态表；已有的不覆盖（本地操作优先）
    fn seed_meta(&mut self, item: &AnswerItem) {
        self.meta.entry(item.id.clone()).or_insert(AnswerMeta {
            vote: item.my_vote,
            voteup: item.voteup,
            faved: false,
        });
    }

    fn open_reading(&mut self) {
        let Some(item) = self.feed.get(self.cursor).cloned() else {
            return;
        };
        // 先用列表里已有的正文秒开，随后由 a 覆盖成权威内容
        self.stream = vec![item.clone()];
        self.index = 0;
        self.total = 1;
        self.scroll = 0;
        self.view = View::Reading;
        self.resume = View::Reading;
        self.read.mark(&item.id);
        self.seed_meta(&item);
        self.rebuild();

        if item.question_id.is_empty() {
            self.status = "single".into();
        } else {
            self.status = "loading…".into();
        }
        self.req = self.req.wrapping_add(1);
        let _ = self.tx.send(Cmd::OpenAnswer {
            qid: item.question_id.clone(),
            aid: item.id.clone(),
            req: self.req,
        });
    }

    fn back_to_list(&mut self) {
        self.view = View::List;
        self.resume = View::List;
        self.status.clear();
    }

    fn step_answer(&mut self, delta: isize) {
        if self.stream.is_empty() {
            return;
        }
        let next = if delta < 0 {
            self.index.saturating_sub(delta.unsigned_abs())
        } else {
            (self.index + delta as usize).min(self.stream.len() - 1)
        };
        if next == self.index {
            self.status = if delta < 0 { "top".into() } else { "bottom".into() };
            return;
        }
        self.index = next;
        // 往下切落到新一条的顶部；往上切落到它的底部，接着往上翻很自然
        self.scroll = if delta < 0 { usize::MAX } else { 0 };
        self.rebuild();
    }

    /// z = 赞，v = 反对；再按同一个键就取消
    fn toggle_vote(&mut self, target: Vote) {
        let Some((aid, kind, my_vote)) = self
            .current()
            .map(|i| (i.id.clone(), i.kind.clone(), i.my_vote))
        else {
            return;
        };
        let key = format!("{aid}:v");
        if self.pending.contains(&key) {
            self.status = "sending…".into();
            return;
        }
        let cur = self.meta.get(&aid).map(|m| m.vote).unwrap_or(my_vote);
        let next = if cur == target { Vote::None } else { target };
        self.pending.insert(key);
        let _ = self.tx.send(Cmd::Vote {
            aid,
            kind,
            vote: next,
        });
    }

    /// c：收藏（已收藏则取消）
    fn toggle_fav(&mut self) {
        let Some((aid, kind)) = self.current().map(|i| (i.id.clone(), i.kind.clone())) else {
            return;
        };
        let key = format!("{aid}:f");
        if self.pending.contains(&key) {
            self.status = "sending…".into();
            return;
        }
        let on = !self.meta.get(&aid).map(|m| m.faved).unwrap_or(false);
        self.pending.insert(key);
        let _ = self.tx.send(Cmd::Favorite { aid, kind, on });
    }

    /// x：评论区
    fn open_comments(&mut self) {
        let Some(item) = self.current() else { return };
        let aid = item.id.clone();
        let kind = item.kind.clone();

        if self.comments.aid != aid {
            self.comments = CommentsState {
                aid: aid.clone(),
                loading: true,
                ..Default::default()
            };
            self.comments.lines = Vec::new();
            let _ = self.tx.send(Cmd::Comments {
                aid,
                kind,
                offset: 0,
            });
        }
        self.view = View::Comments;
        self.resume = View::Comments;
    }

    fn load_more_comments(&mut self) {
        if self.comments.loading || self.comments.items.len() >= self.comments.total {
            return;
        }
        let Some((aid, kind)) = self.current().map(|i| (i.id.clone(), i.kind.clone())) else {
            return;
        };
        self.comments.loading = true;
        let _ = self.tx.send(Cmd::Comments {
            aid,
            kind,
            offset: self.comments.next_offset,
        });
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.body_h as usize)
    }

    fn page(&self) -> usize {
        (self.body_h as usize).max(1)
    }

    /// ↑：还在本条中间就先往回翻一屏，翻到顶才换上一条
    fn page_up_or_prev(&mut self) {
        if self.scroll > 0 {
            self.scroll = self.scroll.saturating_sub(self.page());
        } else {
            self.step_answer(-1);
        }
    }

    /// ↓：还有下文就先翻一屏，翻到底才换下一条
    fn page_down_or_next(&mut self) {
        if self.scroll < self.max_scroll() {
            self.scroll = (self.scroll + self.page()).min(self.max_scroll());
        } else {
            self.step_answer(1);
        }
    }

    /// PgDn/PgUp：只在本条内翻屏，不换条
    fn page_only(&mut self, down: bool) {
        if down {
            self.scroll = (self.scroll + self.page()).min(self.max_scroll());
        } else {
            self.scroll = self.scroll.saturating_sub(self.page());
        }
    }

    // ------------------------------------------------------------- 布局

    /// 终端尺寸变了：清掉已构建宽度，下一帧强制重建
    pub fn on_resize(&mut self) {
        self.built_width = 0;
        self.dirty = true;
    }

    pub fn ensure_layout(&mut self, width: u16) {
        if self.built_width != width {
            self.built_width = width;
            match self.view {
                View::Reading => self.rebuild(),
                View::Comments => self.rebuild_comments(),
                _ => {}
            }
        }
    }

    pub fn rebuild(&mut self) {
        let w = self.built_width.max(24);
        let shown = self.total.max(self.stream.len());
        let meta = self.current().and_then(|it| self.meta.get(&it.id));
        let (header, body) = self
            .stream
            .get(self.index)
            .map(|item| ui::reading::build(item, self.index, shown, meta, w))
            .unwrap_or_default();
        self.header = header;
        self.lines = body;
        let max = self.max_scroll();
        if self.scroll > max {
            self.scroll = max;
        }
    }

    pub fn rebuild_comments(&mut self) {
        let w = self.built_width.max(24);
        self.comments.lines = ui::comments::build(
            &self.comments.items,
            self.comments.total,
            self.comments.loading,
            w,
        );
        let max = self
            .comments
            .lines
            .len()
            .saturating_sub(self.body_h as usize);
        if self.comments.scroll > max {
            self.comments.scroll = max;
        }
    }

    pub fn current_answer(&self) -> Option<(usize, usize)> {
        if self.stream.is_empty() {
            return None;
        }
        Some((self.index, self.total.max(self.stream.len())))
    }

    /// 本条内的页进度（1 起算）；只有一页时返回 None
    pub fn page_progress(&self) -> Option<(usize, usize)> {
        let page = self.page();
        let total = self.lines.len();
        if total <= page {
            return None;
        }
        let pages = (total + page - 1) / page;
        Some((self.scroll / page + 1, pages))
    }

    pub fn comment_progress(&self) -> Option<(usize, usize)> {
        let max = self
            .comments
            .lines
            .len()
            .saturating_sub(self.body_h as usize);
        if max == 0 {
            return None;
        }
        Some((self.comments.scroll.min(max), max))
    }

    // ------------------------------------------------------------- 桥接事件

    pub fn on_bridge(&mut self, ev: BridgeEvent) {
        self.dirty = true;
        match ev {
            BridgeEvent::Status(s) => self.status = s,

            BridgeEvent::Feed(items) => {
                let anchor = self.feed.get(self.cursor).map(|x| x.id.clone());
                self.feed = items;
                // 按 id 找回原来那条，找不到就回顶部
                self.cursor = anchor
                    .and_then(|id| self.feed.iter().position(|x| x.id == id))
                    .unwrap_or(0);
                self.status = format!("{} items", self.feed.len());
            }

            BridgeEvent::Answer { item, req } => {
                if req != self.req {
                    return;
                }
                self.seed_meta(&item);
                if let Some(slot) = self.stream.iter_mut().find(|x| x.id == item.id) {
                    *slot = item;
                } else {
                    let at = self.index.min(self.stream.len());
                    self.stream.insert(at, item);
                }
                self.rebuild();
            }

            BridgeEvent::Answers { items, total, req } => {
                if req != self.req {
                    return;
                }
                for it in &items {
                    self.seed_meta(it);
                }
                self.total = total.max(items.len());
                let cur_id = self
                    .stream
                    .get(self.index)
                    .map(|x| x.id.clone())
                    .unwrap_or_default();

                let mut ordered = items;
                if !cur_id.is_empty() && !ordered.iter().any(|x| x.id == cur_id) {
                    if let Some(cur) = self.stream.get(self.index).cloned() {
                        ordered.insert(0, cur);
                    }
                }
                // 重排时保持当前读到哪儿，不跳回顶部
                let scroll = self.scroll;
                self.stream = ordered;
                self.index = self
                    .stream
                    .iter()
                    .position(|x| x.id == cur_id)
                    .unwrap_or(0);
                self.scroll = scroll;
                self.rebuild();
                self.status = format!("{} answers", self.total);
            }

            BridgeEvent::Voted { aid, vote, count } => {
                let m = self.meta.entry(aid.clone()).or_default();
                m.vote = vote;
                m.voteup = count;
                self.pending.remove(&format!("{aid}:v"));
                self.status = match vote {
                    Vote::Up => format!("voted · {count}"),
                    Vote::Down => format!("downvoted · {count}"),
                    Vote::None => "vote cleared".into(),
                };
                self.rebuild();
            }

            BridgeEvent::Faved { aid, on } => {
                self.pending.remove(&format!("{aid}:f"));
                self.meta.entry(aid).or_default().faved = on;
                self.status = if on { "saved".into() } else { "unsaved".into() };
                self.rebuild();
            }

            BridgeEvent::Comments {
                aid,
                items,
                total,
                next_offset,
            } => {
                if self.comments.aid == aid {
                    for c in items {
                        if !self.comments.items.iter().any(|x| x.id == c.id) {
                            self.comments.items.push(c);
                        }
                    }
                    self.comments.total = total;
                    self.comments.next_offset = next_offset;
                    self.comments.loading = false;
                    self.rebuild_comments();
                }
                self.status = format!("{total} comments");
            }

            BridgeEvent::Error { msg, action } => {
                // 只清掉出错的那一条，别误伤别的请求的在途标记
                match action.as_deref() {
                    Some("comments") => self.comments.loading = false,
                    Some(key) => {
                        self.pending.remove(key);
                    }
                    None => {}
                }
                self.status = format!("× {msg}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ReadLog;

    fn app(tag: &str) -> App {
        let mut a = App::new(Tx::for_test());
        // 别碰真实的家目录；每个测试用独立文件，并行跑也不互相覆盖
        let path = std::env::temp_dir().join(format!(
            "hutui-test-read-{}-{tag}.json",
            std::process::id()
        ));
        a.read = ReadLog::load_from(path);
        a
    }

    fn item(id: &str) -> AnswerItem {
        AnswerItem {
            id: id.into(),
            content: "body".into(),
            ..Default::default()
        }
    }

    /// 修 bug 1：旧编号的 Answers 不能覆盖新打开的那篇
    #[test]
    fn stale_answers_are_dropped() {
        let mut a = app("stale");
        a.feed = vec![item("a")];
        a.open_reading(); // req -> 1，stream = [a]
        let req_now = a.req;

        a.on_bridge(BridgeEvent::Answers {
            items: vec![item("stale-1"), item("stale-2")],
            total: 9,
            req: req_now - 1,
        });
        assert_eq!(a.stream.len(), 1, "旧编号的列表必须被丢掉");

        a.on_bridge(BridgeEvent::Answers {
            items: vec![item("fresh")],
            total: 9,
            req: req_now,
        });
        assert!(a.stream.iter().any(|x| x.id == "fresh"));
    }

    /// 修 bug 5：应急画面下按键不能把正文请回来
    #[test]
    fn panic_swallows_plain_keys() {
        let mut a = app("panic");
        a.feed = vec![item("a")];
        a.open_reading();
        a.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(a.view, View::Panic);

        for k in [KeyCode::Char('q'), KeyCode::Char(' '), KeyCode::Enter, KeyCode::Down] {
            a.on_key(KeyEvent::new(k, KeyModifiers::NONE));
            assert_eq!(a.view, View::Panic, "{k:?} 不该能关掉应急画面");
        }
        assert!(!a.quit);
    }

    /// C 方案：↑↓ 先在本条内翻屏，翻到边界才换条；同一条内吸顶条不变
    #[test]
    fn paging_stays_in_answer_until_boundary() {
        let mut a = app("paging");
        let mut long = item("long");
        long.content = "很长的一段话，用来撑出好几屏。".repeat(200);
        a.stream = vec![long, item("next")];
        a.index = 0;
        a.total = 2;
        a.built_width = 60;
        a.body_h = 8;
        a.rebuild();
        assert!(a.lines.len() > 8, "这条要够长才有翻页");

        let before: Vec<String> = a.header.iter().map(|l| format!("{l:?}")).collect();
        a.page_down_or_next();
        assert_eq!(a.index, 0, "没到底之前不该换条");
        assert!(a.scroll > 0, "应该翻了一屏");
        let after: Vec<String> = a.header.iter().map(|l| format!("{l:?}")).collect();
        assert_eq!(before, after, "同一条里翻屏，吸顶条一个字都不能变");

        for _ in 0..200 {
            a.page_down_or_next();
        }
        assert_eq!(a.index, 1, "翻到底后 ↓ 要换下一条");
        assert_eq!(a.scroll, 0, "往下切换落到新一条顶部");

        // 反向：到顶再 ↑ 换上一条，并落在它的底部
        a.page_up_or_prev();
        assert_eq!(a.index, 0);
        assert_eq!(a.scroll, a.max_scroll(), "往上切换落在上一条底部");
    }

    /// PgUp/PgDn 只翻屏，不换条
    #[test]
    fn page_keys_never_switch_answer() {
        let mut a = app("pageonly");
        let mut long = item("long");
        long.content = "很长的内容。".repeat(300);
        a.stream = vec![long, item("next")];
        a.index = 0;
        a.total = 2;
        a.built_width = 60;
        a.body_h = 8;
        a.rebuild();

        for _ in 0..50 {
            a.page_only(true);
        }
        assert_eq!(a.index, 0, "PgDn 不该换条");
        assert_eq!(a.scroll, a.max_scroll());

        for _ in 0..50 {
            a.page_only(false);
        }
        assert_eq!(a.index, 0);
        assert_eq!(a.scroll, 0);
    }

    /// 修 bug 2：请求失败后 comments.loading 必须解锁
    #[test]
    fn comment_error_unlocks_loading() {
        let mut a = app("comments");
        a.comments.loading = true;
        a.on_bridge(BridgeEvent::Error {
            msg: "boom".into(),
            action: Some("comments".into()),
        });
        assert!(!a.comments.loading);
    }
}
