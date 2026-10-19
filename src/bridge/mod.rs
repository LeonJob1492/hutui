pub mod cdp;
pub mod chrome;
pub mod worker;

use crate::zhihu::{AnswerItem, Comment, Vote};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Instant;

/// 带发送时间戳的命令，用来量「排队等了多久」
#[derive(Debug, Clone)]
pub struct Queued {
    pub cmd: Cmd,
    pub at: Instant,
}

/// 命令口。发出去时自动打时间戳，调用方写法不变。
#[derive(Clone)]
pub struct Tx(Sender<Queued>);

impl Tx {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.0.send(Queued {
            cmd,
            at: Instant::now(),
        });
    }

    #[cfg(test)]
    pub fn for_test() -> Self {
        let (tx, _rx) = channel();
        Tx(tx)
    }
}

/// 主线程 → 工作线程
#[derive(Debug, Clone)]
pub enum Cmd {
    LoadFeed {
        want: usize,
    },
    /// a = 当前这条回答的权威内容，b = 该问题下的回答列表。
    /// `req` 是打开序号：旧序号的事件到主线程后会被丢掉。
    OpenAnswer {
        qid: String,
        aid: String,
        req: u64,
    },
    Vote {
        aid: String,
        kind: String,
        vote: Vote,
    },
    Favorite {
        aid: String,
        kind: String,
        on: bool,
    },
    Comments {
        aid: String,
        kind: String,
        offset: usize,
    },
}

/// 工作线程 → 主线程
#[derive(Debug, Clone)]
pub enum Event {
    Status(String),
    Feed(Vec<AnswerItem>),
    /// 当前这条回答
    Answer {
        item: AnswerItem,
        req: u64,
    },
    /// 该问题下的回答列表（增量）
    Answers {
        items: Vec<AnswerItem>,
        total: usize,
        req: u64,
    },
    Voted {
        aid: String,
        vote: Vote,
        count: i64,
    },
    Faved {
        aid: String,
        on: bool,
    },
    Comments {
        aid: String,
        items: Vec<Comment>,
        total: usize,
        next_offset: usize,
    },
    /// `action` 是出错动作的键（"aid:v" / "aid:f" / "comments"），
    /// 主线程据此只清掉对应那一条在途标记，不误伤别的请求。
    Error {
        msg: String,
        action: Option<String>,
    },
}

/// 起工作线程，返回命令口与事件口
pub fn spawn() -> (Tx, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = channel::<Queued>();
    let (ev_tx, ev_rx) = channel::<Event>();
    std::thread::Builder::new()
        .name("hutui-bridge".into())
        .spawn(move || worker::run(cmd_rx, ev_tx))
        .expect("无法启动桥接线程");
    (Tx(cmd_tx), ev_rx)
}
