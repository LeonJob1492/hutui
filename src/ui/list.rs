use crate::app::App;
use crate::text;
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 每条占 2 行：标题 + 元信息
const ROWS_PER_ITEM: usize = 2;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;

    if app.feed.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  正在拉取推荐流…",
                Style::default().fg(theme::FAINT),
            )),
        ];
        f.render_widget(Paragraph::new(Text::from(lines)), area);
        return;
    }

    let visible = (area.height as usize / ROWS_PER_ITEM).max(1);
    if app.cursor < app.list_offset {
        app.list_offset = app.cursor;
    }
    if app.cursor >= app.list_offset + visible {
        app.list_offset = app.cursor + 1 - visible;
    }
    if app.list_offset + visible > app.feed.len() {
        app.list_offset = app.feed.len().saturating_sub(visible);
    }

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible * ROWS_PER_ITEM);
    for i in app.list_offset..(app.list_offset + visible).min(app.feed.len()) {
        let item = &app.feed[i];
        let active = i == app.cursor;

        let (mark, mark_style, title_style) = if active {
            (
                "▸ ",
                Style::default().fg(theme::ACCENT),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default().fg(theme::FAINT), Style::default().fg(theme::TEXT))
        };

        let title = text::truncate(&text::one_line(item.headline()), width.saturating_sub(3));
        lines.push(Line::from(vec![
            Span::styled(mark.to_string(), mark_style),
            Span::styled(title, title_style),
        ]));

        let mut segs: Vec<String> = Vec::new();
        if !item.author.is_empty() {
            segs.push(item.author.clone());
        }
        match item.kind.as_str() {
            "article" => segs.push("文章".into()),
            "pin" => segs.push("想法".into()),
            _ => {}
        }
        if item.voteup > 0 {
            segs.push(format!("{} 赞", text::compact_num(item.voteup)));
        }
        if item.comments > 0 {
            segs.push(format!("{} 评论", text::compact_num(item.comments)));
        }
        segs.push(text::one_line(&item.excerpt));
        let meta = segs
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");

        let meta = text::truncate(&meta, width.saturating_sub(3));
        lines.push(Line::from(Span::styled(
            format!("  {meta}"),
            Style::default().fg(theme::DIM),
        )));
    }

    f.render_widget(Paragraph::new(Text::from(lines)), area);
}
