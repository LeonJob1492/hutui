use crate::app::App;
use crate::text;
use crate::ui::theme;
use crate::zhihu::Comment;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 评论区也是「一条一屏」之外的独立视图：纯滚动阅读，不带任何操作控件。
pub fn build(items: &[Comment], total: usize, loading: bool, width: u16) -> Vec<Line<'static>> {
    let w = (width as usize).saturating_sub(2).max(20);
    let mut lines: Vec<Line<'static>> = Vec::new();

    let head = format!(" 评论 {}/{} ", items.len(), total.max(items.len()));
    let dashes = w.saturating_sub(text::width(&head) + 2).max(2);
    lines.push(Line::from(Span::styled(
        format!("──{}{}", head, "─".repeat(dashes)),
        Style::default().fg(theme::FAINT),
    )));

    if items.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  还没有评论",
            Style::default().fg(theme::FAINT),
        )));
        return lines;
    }

    for c in items {
        let author = if c.author.is_empty() {
            "匿名".to_string()
        } else {
            c.author.clone()
        };
        let mut spans = vec![
            Span::styled(
                format!("  {author}"),
                Style::default()
                    .fg(theme::DIM)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        let when = text::ago(c.created);
        if !when.is_empty() {
            spans.push(Span::styled(
                format!("  {when}"),
                Style::default().fg(theme::FAINT),
            ));
        }
        if c.voteup > 0 {
            spans.push(Span::styled(
                format!("  ·  {} 赞", text::compact_num(c.voteup)),
                Style::default().fg(theme::FAINT),
            ));
        }
        lines.push(Line::from(spans));

        for l in text::wrap(&c.content, w) {
            if l.is_empty() {
                lines.push(Line::from(""));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {l}"),
                    Style::default().fg(theme::TEXT),
                )));
            }
        }

        if c.children > 0 {
            lines.push(Line::from(Span::styled(
                format!("    └ {} 条回复", c.children),
                Style::default().fg(theme::FAINT),
            )));
        }
        lines.push(Line::from(""));
    }

    if loading {
        lines.push(Line::from(Span::styled(
            "  加载中…",
            Style::default().fg(theme::FAINT),
        )));
    } else if items.len() < total {
        lines.push(Line::from(Span::styled(
            format!("  还有 {} 条 · 按 ↵ 加载更多", total - items.len()),
            Style::default().fg(theme::FAINT),
        )));
    }

    lines
}

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    if app.comments.lines.is_empty() {
        return;
    }
    let height = area.height as usize;
    let max = app.comments.lines.len().saturating_sub(height);
    if app.comments.scroll > max {
        app.comments.scroll = max;
    }

    let visible: Vec<Line<'static>> = app
        .comments
        .lines
        .iter()
        .skip(app.comments.scroll)
        .take(height)
        .cloned()
        .collect();
    f.render_widget(Paragraph::new(Text::from(visible)), area);
}
