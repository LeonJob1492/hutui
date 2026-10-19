use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

/// 最多记住多少条已读，超出丢最早的
const MAX_IDS: usize = 800;

/// 「看过的」记录：内存查得快，落盘保证重启后还认。
pub struct ReadLog {
    path: PathBuf,
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl ReadLog {
    pub fn load() -> Self {
        Self::load_from(log_path())
    }

    /// 指定文件的版本，测试与迁移时用
    pub fn load_from(path: PathBuf) -> Self {
        let mut set = HashSet::new();
        let mut order = VecDeque::new();

        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(&text) {
                for id in ids {
                    if set.insert(id.clone()) {
                        order.push_back(id);
                    }
                }
            }
        }

        Self { path, set, order }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.set.contains(id)
    }

    pub fn mark(&mut self, id: &str) {
        if id.is_empty() || !self.set.insert(id.to_string()) {
            return;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > MAX_IDS {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.save();
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let ids: Vec<&String> = self.order.iter().collect();
        if let Ok(text) = serde_json::to_string(&ids) {
            let _ = std::fs::write(&self.path, text);
        }
    }
}

fn log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".hutui").join("read.json")
}
