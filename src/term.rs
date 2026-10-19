use anyhow::Result;
use crossterm::{
    cursor,
    event::{self},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{stdout, Stdout, Write};
use std::time::Duration;

use crate::config;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

fn write_osc_title(title: &str) {
    let mut out = stdout();
    let _ = write!(out, "\x1b]2;{title}\x07");
    let _ = out.flush();
}

/// 初始化终端；同时装 panic hook，保证异常退出后终端能恢复
pub fn install() -> Result<Tui> {
    terminal::enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, cursor::Hide)?;
    out.flush()?;

    // 伪装标题：Ghostty 已设 no-title，precmd 在前台程序运行时不会覆盖
    write_osc_title(config::APP_TITLE);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        default_hook(info);
    }));

    let mut t = Terminal::new(CrosstermBackend::new(stdout()))?;
    t.clear()?;
    Ok(t)
}

pub fn restore() -> Result<()> {
    let mut out = stdout();
    // 标题恢复成目录名，兜底（zsh 的 precmd 也会写一次）
    let dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "~".into());
    let _ = write!(out, "\x1b]2;{dir}\x07");
    execute!(out, LeaveAlternateScreen, cursor::Show)?;
    out.flush()?;
    terminal::disable_raw_mode()?;
    Ok(())
}

/// 等一个终端事件（按键、尺寸变化…），超时返回 None
pub fn poll_event(timeout: Duration) -> Result<Option<event::Event>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    Ok(Some(event::read()?))
}
