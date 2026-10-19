use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::bridge::{Cmd, Event as BridgeEvent};
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
    pub lines: Vec<Line<'static>>,
    pub scroll: usize,
    scroll_by_id: HashMap<String, usize>,

    /// 每条内容上我的表态（赞/收藏）
    pub meta: HashMap<String, AnswerMeta>,
    pub comments: CommentsState,

    // ---- 布局
    pub built_width: u16,
    pub body_h: u16,

    pub status: String,
    pub quit: bool,
    last_esc: Option<Instant>,
    tx: Sender<Cmd>,
}

impl App {
    pub fn new(tx: Sender<Cmd>) -> Self {
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
            lines: Vec::new(),
            scroll: 0,
            scroll_by_id: HashMap::new(),
            meta: HashMap::new(),
            comments: CommentsState::default(),
            built_width: 0,
            body_h: 0,
            status: "连接中…".into(),
            quit: false,
            last_esc: None,
            tx,
        }
    }

    // ------------------------------------------------------------- 按键

    pub fn on_key(&mut self, k: KeyEvent) {
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
            View::Panic => {
                self.view = self.resume;
                self.rebuild();
            }
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
                self.status = "刷新…".into();
                let _ = self.tx.send(Cmd::LoadFeed {
                    want: config::FEED_TARGET,
                });
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn on_key_reading(&mut self, k: KeyEvent) {
        let page = (self.body_h.saturating_sub(1) as usize).max(1);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.step_answer(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step_answer(1),
            KeyCode::Char('n') => self.step_answer(1),
            KeyCode::Char('p') => self.step_answer(-1),
            KeyCode::Char('z') => self.toggle_vote(Vote::Up),
            KeyCode::Char('v') => self.toggle_vote(Vote::Down),
            KeyCode::Char('c') => self.toggle_fav(),
            KeyCode::Char('x') => self.open_comments(),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown => self.scroll_by(page),
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
            self.status = "单篇".into();
        } else {
            self.status = "拉取回答…".into();
        }
        let _ = self.tx.send(Cmd::OpenAnswer {
            qid: item.question_id.clone(),
            aid: item.id.clone(),
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
        let cur_id = self.stream[self.index].id.clone();
        self.scroll_by_id.insert(cur_id, self.scroll);

        let next = if delta < 0 {
            self.index.saturating_sub(delta.unsigned_abs())
        } else {
            (self.index + delta as usize).min(self.stream.len() - 1)
        };
        if next == self.index {
            self.status = if delta < 0 {
                "已是第一条".into()
            } else {
                "已是最后一条".into()
            };
            return;
        }
        self.index = next;
        self.scroll = *self.scroll_by_id.get(&self.stream[next].id).unwrap_or(&0);
        self.rebuild();
    }

    /// z = 赞，v = 反对；再按同一个键就取消
    fn toggle_vote(&mut self, target: Vote) {
        let Some(item) = self.current() else { return };
        let cur = self
            .meta
            .get(&item.id)
            .map(|m| m.vote)
            .unwrap_or(item.my_vote);
        let next = if cur == target { Vote::None } else { target };
        let _ = self.tx.send(Cmd::Vote {
            aid: item.id.clone(),
            kind: item.kind.clone(),
            vote: next,
        });
    }

    /// c：收藏（已收藏则取消）
    fn toggle_fav(&mut self) {
        let Some(item) = self.current() else { return };
        let on = !self.meta.get(&item.id).map(|m| m.faved).unwrap_or(false);
        let _ = self.tx.send(Cmd::Favorite {
            aid: item.id.clone(),
            kind: item.kind.clone(),
            on,
        });
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

    fn scroll_by(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.max_scroll());
    }

    // ------------------------------------------------------------- 布局

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
        self.lines = self
            .stream
            .get(self.index)
            .map(|item| ui::reading::build(item, self.index, shown, meta, w))
            .unwrap_or_default();
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

    pub fn scroll_progress(&self) -> Option<(usize, usize)> {
        let max = self.max_scroll();
        if max == 0 {
            return None;
        }
        Some((self.scroll.min(max), max))
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
        match ev {
            BridgeEvent::Status(s) => self.status = s,

            BridgeEvent::Feed(items) => {
                self.feed = items;
                if self.cursor >= self.feed.len() {
                    self.cursor = self.feed.len().saturating_sub(1);
                }
                self.status = format!("{} items", self.feed.len());
            }

            BridgeEvent::Answer(item) => {
                self.seed_meta(&item);
                if let Some(slot) = self.stream.iter_mut().find(|x| x.id == item.id) {
                    *slot = item;
                } else {
                    let at = self.index.min(self.stream.len());
                    self.stream.insert(at, item);
                }
                self.rebuild();
            }

            BridgeEvent::Answers { items, total } => {
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
                let scroll = self.scroll_by_id.get(&cur_id).copied().unwrap_or(0);
                self.stream = ordered;
                self.index = self
                    .stream
                    .iter()
                    .position(|x| x.id == cur_id)
                    .unwrap_or(0);
                self.scroll = scroll;
                self.rebuild();
                self.status = format!("{} 条回答", self.total);
            }

            BridgeEvent::Voted { aid, vote, count } => {
                let m = self.meta.entry(aid).or_default();
                m.vote = vote;
                m.voteup = count;
                self.status = match vote {
                    Vote::Up => format!("已赞 · {count}"),
                    Vote::Down => format!("已反对 · {count}"),
                    Vote::None => "已取消赞同".into(),
                };
                self.rebuild();
            }

            BridgeEvent::Faved { aid, on } => {
                self.meta.entry(aid).or_default().faved = on;
                self.status = if on {
                    "已收藏".into()
                } else {
                    "已取消收藏".into()
                };
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
                self.status = format!("{} 条评论", total);
            }

            BridgeEvent::Error(e) => self.status = format!("× {e}"),
        }
    }
}
