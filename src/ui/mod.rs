pub mod list;
pub mod panic;
pub mod reading;
pub mod theme;

use crate::app::{App, View};
use crate::text;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.ensure_layout(area.width);

    // panic 必须整体接管视口，不留任何正文残留
    if app.view == View::Panic {
        f.render_widget(
            Paragraph::new(Text::from(panic::build(area.width))),
            area,
        );
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    header(f, app, rows[0]);
    app.body_h = rows[1].height;
    match app.view {
        View::List => list::draw(f, app, rows[1]),
        View::Reading => reading::draw(f, app, rows[1]),
        View::Panic => {}
    }
    footer(f, app, rows[2]);
}

fn header(f: &mut Frame, app: &App, area: Rect) {
    let bold = Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme::DIM);
    let faint = Style::default().fg(theme::FAINT);

    let mut spans = vec![
        Span::styled(" ▌ ", Style::default().fg(theme::ACCENT)),
        Span::styled("Codex", bold),
        Span::styled("  ~/hutui", dim),
        Span::styled("  main*", faint),
        Span::styled("  rustc 1.98", faint),
    ];

    if let Some((idx, total)) = app.current_block() {
        if app.view == View::Reading {
            spans.push(Span::styled(format!("   {}/{}", idx + 1, total), faint));
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer(f: &mut Frame, app: &App, area: Rect) {
    let status = text::truncate(&app.status, area.width.saturating_sub(3) as usize);
    let style = if app.status.starts_with('×') {
        Style::default().fg(theme::WARN)
    } else {
        Style::default().fg(theme::DIM)
    };
    let line = Line::from(vec![Span::styled("  ", Style::default()), Span::styled(status, style)]);
    f.render_widget(Paragraph::new(line), area);
}
