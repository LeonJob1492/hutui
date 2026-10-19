pub mod list;
pub mod panic;
pub mod reading;
pub mod theme;

use crate::app::{App, View};
use crate::config;
use crate::text;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    // 整体宽度上限 1024px（折算成列），超出部分居中留白
    let area = centered(f.area());
    app.ensure_layout(area.width);

    // panic 必须整体接管视口，不留任何正文残留
    if app.view == View::Panic {
        f.render_widget(Paragraph::new(Text::from(panic::build(area.width))), area);
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

/// 把视口收窄到最大宽度并水平居中
fn centered(area: Rect) -> Rect {
    let w = area.width.min(config::MAX_LAYOUT_COLS);
    if w >= area.width {
        return area;
    }
    let x = area.x + (area.width - w) / 2;
    Rect::new(x, area.y, w, area.height)
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

    if app.view == View::Reading {
        if let Some((idx, total)) = app.current_answer() {
            spans.push(Span::styled(format!("   {}/{}", idx + 1, total), faint));
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn footer(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(14)]).split(area);

    let status = text::truncate(&app.status, cols[0].width.saturating_sub(3) as usize);
    let style = if app.status.starts_with('×') {
        Style::default().fg(theme::WARN)
    } else {
        Style::default().fg(theme::DIM)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(status, style),
        ])),
        cols[0],
    );

    // 右上角给条内滚动进度，避免长回答里不知道自己在哪
    let right = match (app.view, app.scroll_progress()) {
        (View::Reading, Some((cur, max))) => format!("{cur}/{max} "),
        _ => String::new(),
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            right,
            Style::default().fg(theme::FAINT),
        )))
        .alignment(Alignment::Right),
        cols[1],
    );
}
