use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::bridge::{Cmd, Event as BridgeEvent};
use crate::config;
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
    pub feed: Vec<AnswerItem>,
    pub cursor: usize,
    pub list_offset: usize,
    pub stream: Vec<AnswerItem>,
    pub lines: Vec<Line<'static>>,
    pub block_starts: Vec<usize>,
    pub scroll: usize,
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
            stream: Vec::new(),
            lines: Vec::new(),
            block_starts: Vec::new(),
            scroll: 0,
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

    fn on_key_reading(&mut self, k: KeyEvent) {
        let page = (self.body_h.saturating_sub(1) as usize).max(1);
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(page),
            KeyCode::PageDown => self.scroll_by(page),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.lines.len().saturating_sub(self.body_h as usize),
            KeyCode::Char('n') => self.jump_block(1),
            KeyCode::Char('p') => self.jump_block(-1),
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
        self.stream = vec![item.clone()];
        self.scroll = 0;
        self.view = View::Reading;
        self.resume = View::Reading;
        self.rebuild();

        if item.question_id.is_empty() {
            self.status = "单篇".into();
        } else {
            self.status = "拉同问题回答…".into();
            let _ = self.tx.send(Cmd::LoadAnswers {
                qid: item.question_id.clone(),
            });
        }
    }

    fn back_to_list(&mut self) {
        self.view = View::List;
        self.resume = View::List;
        self.status.clear();
    }

    fn scroll_by(&mut self, n: usize) {
        let max = self.lines.len().saturating_sub(self.body_h as usize);
        self.scroll = (self.scroll + n).min(max);
    }

    fn jump_block(&mut self, dir: i32) {
        if self.block_starts.is_empty() {
            return;
        }
        let after = self
            .block_starts
            .iter()
            .position(|&s| s > self.scroll)
            .unwrap_or(self.block_starts.len());
        let idx = if dir >= 0 {
            after.min(self.block_starts.len() - 1)
        } else {
            after.saturating_sub(2)
        };
        self.scroll = self.block_starts[idx];
    }

    // ------------------------------------------------------------- 布局

    /// 宽度变化时重建阅读流（换行结果依赖宽度）
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
        let (lines, starts) = ui::reading::build(&self.stream, w);
        self.lines = lines;
        self.block_starts = starts;
        let max = self.lines.len().saturating_sub(self.body_h as usize);
        if self.scroll > max {
            self.scroll = max;
        }
    }

    /// 当前读到第几条（给 header 用）
    pub fn current_block(&self) -> Option<(usize, usize)> {
        if self.block_starts.is_empty() {
            return None;
        }
        let idx = self
            .block_starts
            .iter()
            .rposition(|&s| s <= self.scroll)
            .unwrap_or(0);
        Some((idx, self.block_starts.len()))
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
            BridgeEvent::Answers { items, is_end } => {
                let mut added = 0usize;
                for it in items {
                    if self.stream.len() >= config::STREAM_MAX {
                        break;
                    }
                    if it.content.is_empty() || self.stream.iter().any(|x| x.id == it.id) {
                        continue;
                    }
                    self.stream.push(it);
                    added += 1;
                }
                if added > 0 {
                    self.rebuild();
                }
                self.status = if is_end {
                    format!("{} 条回答 · 已到底", self.stream.len())
                } else {
                    format!("{} 条回答", self.stream.len())
                };
            }
            BridgeEvent::Error(e) => self.status = format!("× {e}"),
        }
    }
}
