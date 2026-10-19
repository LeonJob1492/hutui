mod app;
mod bridge;
mod config;
mod state;
mod term;
mod text;
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
        terminal.draw(|f| ui::draw(f, app))?;

        if let Some(key) = term::poll_key(Duration::from_millis(110))? {
            app.on_key(key);
        }
        while let Ok(ev) = ev_rx.try_recv() {
            app.on_bridge(ev);
        }

        if app.quit {
            return Ok(());
        }
    }
}
