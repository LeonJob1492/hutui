use crate::app::App;
use crate::text;
use crate::ui::theme;
use crate::zhihu::AnswerItem;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 把**当前这一条**回答摊平成行。
///
/// 阅读视图是「一条一屏」：↑↓ 换回答，条内滚动靠 PgUp/PgDn。
pub fn build(item: &AnswerItem, index: usize, total: usize, width: u16) -> Vec<Line<'static>> {
    let w = (width as usize).saturating_sub(2).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let author = if item.author.is_empty() {
        "匿名".to_string()
    } else {
        item.author.clone()
    };
    let head = format!(
        " {}/{}  ·  {}  ·  {} 赞 ",
        index + 1,
        total,
        author,
        text::compact_num(item.voteup)
    );
    let dashes = w.saturating_sub(text::width(&head) + 2).max(2);
    lines.push(Line::from(Span::styled(
        format!("──{}{}", head, "─".repeat(dashes)),
        Style::default().fg(theme::FAINT),
    )));

    if !item.question_title.is_empty() {
        let t = text::truncate(&text::one_line(&item.question_title), w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {t}"),
            Style::default()
                .fg(theme::DIM)
                .add_modifier(Modifier::BOLD),
        )));
    }

    let body = if item.content.is_empty() {
        item.excerpt.clone()
    } else {
        item.content.clone()
    };
    for l in text::wrap(&body, w) {
        if l.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(theme::TEXT),
            )));
        }
    }

    lines
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    if app.lines.is_empty() {
        return;
    }
    let height = area.height as usize;
    let max = app.lines.len().saturating_sub(height);
    if app.scroll > max {
        app.scroll = max;
    }

    let visible: Vec<Line<'static>> = app
        .lines
        .iter()
        .skip(app.scroll)
        .take(height)
        .cloned()
        .collect();
    f.render_widget(Paragraph::new(Text::from(visible)), area);
}
