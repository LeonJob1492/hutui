use std::path::PathBuf;

/// Chrome 调试端口
pub const CDP_PORT: u16 = 9222;
/// 后台窗口尺寸
pub const WINDOW_SIZE: &str = "1100,760";
/// 伪装窗口标题（OSC 2）
pub const APP_TITLE: &str = "handler.rs — gateway";
/// 列表首次预取目标条数
pub const FEED_TARGET: usize = 42;
/// 列表最多补几页
pub const FEED_PAGE_SCROLLS: usize = 9;
/// 同问题回答最多补几页
pub const ANSWER_PAGE_SCROLLS: usize = 4;
/// 首屏捕获窗口
pub const FEED_TIMEOUT_MS: u64 = 11_000;
/// 每个补页捕获窗口
pub const PAGE_TIMEOUT_MS: u64 = 2_800;
/// 阅读流硬上限
pub const STREAM_MAX: usize = 50;

/// 界面整体最大宽度。终端按字符列计，这里按 8px/字符折算：
/// 1024px ÷ 8 = 128 列。要改宽度只动 MAX_LAYOUT_PX。
pub const MAX_LAYOUT_PX: u16 = 1024;
pub const CELL_PX: u16 = 8;
pub const MAX_LAYOUT_COLS: u16 = MAX_LAYOUT_PX / CELL_PX;

pub fn profile_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".hutui").join("chrome-profile")
}
