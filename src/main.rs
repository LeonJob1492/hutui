mod app;
mod bridge;
mod config;
mod state;
mod term;
mod text;
mod trace;
mod ui;
mod zhihu;

use anyhow::Result;
use std::sync::mpsc::Receiver;
use std::time::Duration;

fn main() -> Result<()> {
    let (cmd_tx, ev_rx) = bridge::spawn();
    let mut app = app::App::new(cmd_tx.clone());
    let mut terminal = term::install()?;

    let _ = cmd_tx.send(bridge::Cmd::LoadFeed {
        want: config::FEED_TARGET,
    });

    let result = run(&mut terminal, &mut app, &ev_rx);
    let _ = term::restore();
    result
}

fn run(
    terminal: &mut term::Tui,
    app: &mut app::App,
    ev_rx: &Receiver<bridge::Event>,
) -> Result<()> {
    loop {
        // 只在有变化时重绘：闲下来就完全不碰终端
        if app.dirty {
            terminal.draw(|f| ui::draw(f, app))?;
            app.dirty = false;
        }

        if let Some(ev) = term::poll_event(Duration::from_millis(120))? {
            match ev {
                crossterm::event::Event::Key(key) => app.on_key(key),
                crossterm::event::Event::Resize(_, _) => app.on_resize(),
                _ => {}
            }
        }
        while let Ok(ev) = ev_rx.try_recv() {
            app.on_bridge(ev);
        }

        if app.quit {
            return Ok(());
        }
    }
}
