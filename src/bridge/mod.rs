pub mod cdp;
pub mod chrome;
pub mod worker;

use crate::zhihu::AnswerItem;
use std::sync::mpsc::{channel, Receiver, Sender};

/// 主线程 → 工作线程
#[derive(Debug, Clone)]
pub enum Cmd {
    LoadFeed { want: usize },
    /// a = 当前这条回答的权威内容，b = 该问题下的回答列表
    OpenAnswer { qid: String, aid: String },
}

/// 工作线程 → 主线程
#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    Feed(Vec<AnswerItem>),
    /// 当前这条回答
    Answer(AnswerItem),
    /// 该问题下的回答列表（增量）
    Answers { items: Vec<AnswerItem>, total: usize },
    Error(String),
}

/// 起工作线程，返回命令口与事件口
pub fn spawn() -> (Sender<Cmd>, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = channel::<Cmd>();
    let (ev_tx, ev_rx) = channel::<Event>();
    std::thread::Builder::new()
        .name("hutui-bridge".into())
        .spawn(move || worker::run(cmd_rx, ev_tx))
        .expect("无法启动桥接线程");
    (cmd_tx, ev_rx)
}
