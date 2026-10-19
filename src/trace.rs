//! 极简计时日志：写到 `~/.hutui/trace.log`，用来定位「这一步到底慢在哪」。
//!
//! 只记时间与动作，不记正文、不记 cookie。写失败一律忽略——调试设施不该影响主流程。

use std::io::Write;
use std::path::PathBuf;

/// 超过这个大小就清空重来，避免日志无限长
const MAX_BYTES: u64 = 2 * 1024 * 1024;

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
    // 涨到上限就清空，第一行留个标记
    if std::fs::metadata(path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::write(path, b"--- trace rotated ---\n");
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


fn path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".hutui").join("trace.log"))
}

/// 当前墙钟毫秒（和页面里的 Date.now() 同一基准，可直接相减）
pub fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 把耗时格式化毫秒，方便直接拼进日志
pub fn ms(d: std::time::Duration) -> String {
    format!("{:.0}ms", d.as_secs_f64() * 1000.0)
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
