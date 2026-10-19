use anyhow::{bail, Result};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::config;

pub fn port_alive(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(400)).is_ok()
}

/// 确保有一个带调试端口的独立 Chrome 实例在跑。
///
/// 关键约束：Chrome ≥136 起，使用默认 user-data-dir 时会直接忽略
/// `--remote-debugging-port`，所以必须用独立 profile。
/// 用 `open -n -g` 启动：新实例 + 不抢焦点。
pub fn ensure(port: u16) -> Result<()> {
    if port_alive(port) {
        return Ok(());
    }

    let dir = config::profile_dir();
    std::fs::create_dir_all(&dir)?;

    let mut args: Vec<String> = vec![
        "-n".into(),
        "-g".into(),
        "-a".into(),
        "Google Chrome".into(),
        "--args".into(),
        format!("--user-data-dir={}", dir.display()),
        format!("--remote-debugging-port={port}"),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--disable-sync".into(),
        format!("--window-size={}", config::WINDOW_SIZE),
        "about:blank".into(),
    ];

    let status = Command::new("open").args(&mut args).status()?;
    if !status.success() {
        bail!("open 启动 Chrome 失败（退出码 {:?}）", status.code());
    }

    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        if port_alive(port) {
            // 端口通了再多等一拍，让 target 列表稳定
            std::thread::sleep(Duration::from_millis(700));
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    bail!("Chrome 调试端口 {port} 在 25s 内没起来")
}
