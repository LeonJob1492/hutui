use crate::app::App;
use crate::text;
use crate::ui::theme;
use crate::zhihu::{AnswerItem, AnswerMeta, Vote};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 把**当前这一条**回答摊平成行。
///
/// 阅读视图是「一条一屏」：↑↓ 换回答，条内滚动靠 PgUp/PgDn。
pub fn build(
    item: &AnswerItem,
    index: usize,
    total: usize,
    meta: Option<&AnswerMeta>,
    width: u16,
) -> Vec<Line<'static>> {
    let w = (width as usize).saturating_sub(2).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ① 位置行：只放第几条，不再塞作者信息
    let head = format!(" {}/{} ", index + 1, total);
    let dashes = w.saturating_sub(text::width(&head) + 2).max(2);
    lines.push(Line::from(Span::styled(
        format!("──{}{}", head, "─".repeat(dashes)),
        Style::default().fg(theme::FAINT),
    )));

    // ② 问题标题
    if !item.question_title.is_empty() {
        let t = text::truncate(&text::one_line(&item.question_title), w.saturating_sub(4));
        lines.push(Line::from(Span::styled(
            format!("  {t}"),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // ③ 次要信息：作者 · 赞数 · 表态 · 时间，统一走 META 色
    let author = if item.author.is_empty() {
        "匿名".to_string()
    } else {
        item.author.clone()
    };
    let count = meta.map(|m| m.voteup).filter(|c| *c > 0).unwrap_or(item.voteup);
    let (vote, faved) = meta.map(|m| (m.vote, m.faved)).unwrap_or((item.my_vote, false));
    let when = text::datetime(item.created);

    let meta_style = Style::default().fg(theme::META);
    let mut spans: Vec<Span<'static>> = vec![Span::styled(format!("  {author}"), meta_style)];
    if count > 0 {
        spans.push(Span::styled(
            format!("  ·  {} 赞", text::compact_num(count)),
            meta_style,
        ));
    }
    match vote {
        Vote::Up => spans.push(Span::styled("  ✓", Style::default().fg(theme::ACCENT))),
        Vote::Down => spans.push(Span::styled("  ✗", Style::default().fg(theme::WARN))),
        Vote::None => {}
    }
    if faved {
        spans.push(Span::styled(
            "  ★",
            Style::default()
                .fg(theme::META)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if !when.is_empty() {
        spans.push(Span::styled(format!("  ·  {when}"), meta_style));
    }
    lines.push(Line::from(spans));
    lines.push(Line::from(""));

    // ④ 正文
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


