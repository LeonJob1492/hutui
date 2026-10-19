use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::bridge::{Cmd, Event as BridgeEvent};
use crate::config;
use crate::state::ReadLog;
use crate::ui;
use crate::zhihu::AnswerItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Reading,
    Panic,
}

pub struct App {
    pub view: View,
    /// 从 panic 恢复时回到哪个视图
    pub resume: View,

    // ---- 列表
    pub feed: Vec<AnswerItem>,
    pub cursor: usize,
    pub list_offset: usize,
    /// 列表可视区上方 / 下方还剩多少条，给 footer 提示用
    pub list_above: usize,
    pub list_below: usize,
    /// 看过的条目
    pub read: ReadLog,

    // ---- 阅读：一条一屏，↑↓ 换条
    pub stream: Vec<AnswerItem>,
    pub index: usize,
    /// 该问题下回答总数（接口给的 paging.totals）
    pub total: usize,
    /// 当前这条回答摊平后的行
    pub lines: Vec<Line<'static>>,
    /// 当前这条回答内的滚动位置
    pub scroll: usize,
    /// 按回答 id 记住各自的滚动位置，切回来不丢
    scroll_by_id: HashMap<String, usize>,

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

    /// 阅读视图：↑↓ 换回答；单条比一屏长时用 PgUp/PgDn 在条内滚。
    fn on_key_reading(&mut self, k: KeyEvent) {
        let page = (self.body_h.saturating_sub(1) as usize).max(1);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.step_answer(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step_answer(1),
            KeyCode::Char('n') => self.step_answer(1),
            KeyCode::Char('p') => self.step_answer(-1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),
            KeyCode::Left | KeyCode::Backspace => self.back_to_list(),
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

    /// 切换到上/下一条回答，顺带还原那条自己的滚动位置
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

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.body_h as usize)
    }

    fn scroll_by(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.max_scroll());
    }

    // ------------------------------------------------------------- 布局

    /// 宽度变化时重建当前回答的行（换行结果依赖宽度）
    pub fn ensure_layout(&mut self, width: u16) {
        if self.built_width != width {
            self.built_width = width;
            if self.view == View::Reading {
                self.rebuild();
            }
        }
    }

    pub fn rebuild(&mut self) {
        let w = self.built_width.max(24);
        let shown = self.total.max(self.stream.len());
        self.lines = self
            .stream
            .get(self.index)
            .map(|item| ui::reading::build(item, self.index, shown, w))
            .unwrap_or_default();
        let max = self.max_scroll();
        if self.scroll > max {
            self.scroll = max;
        }
    }

    /// 当前读到第几条（给 header 用）：(从 0 开始的序号, 总数)
    pub fn current_answer(&self) -> Option<(usize, usize)> {
        if self.stream.is_empty() {
            return None;
        }
        Some((self.index, self.total.max(self.stream.len())))
    }

    /// 条内滚动进度：(已滚, 总可滚)，给 footer 用
    pub fn scroll_progress(&self) -> Option<(usize, usize)> {
        let max = self.max_scroll();
        if max == 0 {
            return None;
        }
        Some((self.scroll.min(max), max))
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

            // a：当前这条回答的权威内容
            BridgeEvent::Answer(item) => {
                if let Some(slot) = self.stream.iter_mut().find(|x| x.id == item.id) {
                    *slot = item;
                } else {
                    let at = self.index.min(self.stream.len());
                    self.stream.insert(at, item);
                }
                self.rebuild();
            }

            // b：该问题下的回答列表，按接口顺序重排并把当前这条定位到它自己的位置
            BridgeEvent::Answers { items, total } => {
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

            BridgeEvent::Error(e) => self.status = format!("× {e}"),
        }
    }
}
