use std::path::PathBuf;

/// Chrome 调试端口
pub const CDP_PORT: u16 = 9222;
/// 后台窗口尺寸
pub const WINDOW_SIZE: &str = "1100,760";
/// 伪装窗口标题（OSC 2）
pub const APP_TITLE: &str = "handler.rs — gateway";

/// 列表预取目标条数
pub const FEED_TARGET: usize = 42;
/// 推荐流最多跟几页（每页 6 条）
pub const FEED_MAX_PAGES: usize = 12;
/// 触底续拉时一次多要多少条
pub const FEED_MORE_HINT: usize = 24;
/// 推荐流列表的硬上限，防止无限堆积
pub const FEED_HARD_MAX: usize = 300;
/// 回答列表每页条数
pub const ANSWERS_PAGE: usize = 20;
/// 评论每页条数
pub const COMMENTS_PAGE: usize = 20;
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
