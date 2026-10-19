//! 极简计时日志：写到 `~/.hutui/trace.log`，用来定位「这一步到底慢在哪」。
//!
//! 只记时间与动作，不记正文、不记 cookie。写失败一律忽略——调试设施不该影响主流程。

use std::io::Write;
use std::path::PathBuf;

/// 记一行。调用点都在工作线程，O_APPEND 追加足够安全。
pub fn log(msg: &str) {
    if let Some(path) = path() {
        log_to(&path, msg);
    }
}

fn log_to(path: &std::path::Path, msg: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let now = chrono::Local::now().format("%H:%M:%S%.3f");
    let _ = writeln!(f, "{now} {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_one_line() {
        let dir = std::env::temp_dir().join(format!("hutui-trace-{}", std::process::id()));
        let path = dir.join("trace.log");
        let _ = std::fs::remove_file(&path);
        log_to(&path, "POST /voters cdp=12ms fetch=9ms");
        log_to(&path, "cmd Vote queued=1ms");
        let text = std::fs::read_to_string(&path).expect("日志应写成功");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("POST /voters cdp=12ms fetch=9ms"));
        assert!(lines[1].ends_with("cmd Vote queued=1ms"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".hutui").join("trace.log"))
}

/// 把耗时格式化成毫秒，方便直接拼进日志
pub fn ms(d: std::time::Duration) -> String {
    format!("{:.0}ms", d.as_secs_f64() * 1000.0)
}
