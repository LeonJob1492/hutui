use crate::app::App;
use crate::text;
use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 每条 2 行内容 + 1 行留白，行间留气口
const ROWS_PER_ITEM: usize = 3;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width as usize;

    if app.feed.is_empty() {
        app.list_above = 0;
        app.list_below = 0;
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

    let start = app.list_offset;
    let end = (start + visible).min(app.feed.len());
    app.list_above = start;
    app.list_below = app.feed.len().saturating_sub(end);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity((end - start) * ROWS_PER_ITEM);

    for i in start..end {
        let item = &app.feed[i];
        let active = i == app.cursor;
        let read = app.read.contains(&item.id);

        // 行首标记：光标 ▸ / 读过 · / 未读留白
        let (mark, mark_style) = if active {
            ("▸ ", Style::default().fg(theme::ACCENT))
        } else if read {
            ("· ", Style::default().fg(theme::FAINT))
        } else {
            ("  ", Style::default().fg(theme::FAINT))
        };

        let title_style = if active {
            Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
        } else if read {
            Style::default().fg(theme::FAINT)
        } else {
            Style::default().fg(theme::TEXT)
        };

        let title = text::truncate(&text::one_line(item.headline()), width.saturating_sub(3));
        lines.push(Line::from(vec![
            Span::styled(mark.to_string(), mark_style),
            Span::styled(title, title_style),
        ]));

        // 次行：作者/数据/摘要，摘要再压一档颜色，形成三级层次
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
        let head = segs.join(" · ");
        let head_style = if read {
            Style::default().fg(theme::FAINT)
        } else {
            Style::default().fg(theme::DIM)
        };

        let avail = width.saturating_sub(3);
        let excerpt = text::one_line(&item.excerpt);
        let mut spans = vec![Span::styled("  ", Style::default())];

        let head_w = text::width(&head);
        if head.is_empty() {
            spans.push(Span::styled(
                text::truncate(&excerpt, avail),
                Style::default().fg(theme::FAINT),
            ));
        } else if head_w + 6 < avail {
            spans.push(Span::styled(head, head_style));
            if !excerpt.is_empty() {
                spans.push(Span::styled("  ·  ", Style::default().fg(theme::FAINT)));
                spans.push(Span::styled(
                    text::truncate(&excerpt, avail - head_w - 5),
                    Style::default().fg(theme::FAINT),
                ));
            }
        } else {
            spans.push(Span::styled(text::truncate(&head, avail), head_style));
        }
        lines.push(Line::from(spans));

        lines.push(Line::from(""));
    }

    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

