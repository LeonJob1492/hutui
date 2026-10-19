use crate::app::App;
use crate::text;
use crate::ui::theme;
use crate::zhihu::{AnswerItem, AnswerMeta, Vote};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// 把当前这条回答拆成「吸顶信息条」和「可滚动的正文」两段。
///
/// 吸顶条（分隔行 / 标题 / 作者·赞·时间）不参与滚动，所以翻屏时你始终看得见
/// `3/15` 和作者——同一条里翻页这几行不动，切换回答才变，一眼能分辨。
pub fn build(
    item: &AnswerItem,
    index: usize,
    total: usize,
    meta: Option<&AnswerMeta>,
    width: u16,
) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let w = (width as usize).saturating_sub(2).max(20);
    let mut header: Vec<Line<'static>> = Vec::new();

    // ① 位置行
    let head = format!(" {}/{} ", index + 1, total);
    let dashes = w.saturating_sub(text::width(&head) + 2).max(2);
    header.push(Line::from(Span::styled(
        format!("──{}{}", head, "─".repeat(dashes)),
        Style::default().fg(theme::FAINT),
    )));

    // ② 问题标题
    if !item.question_title.is_empty() {
        let t = text::truncate(&text::one_line(&item.question_title), w.saturating_sub(4));
        header.push(Line::from(Span::styled(
            format!("  {t}"),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // ③ 次要信息：作者 · 赞数 · 表态 · 时间
    let author = if item.author.is_empty() {
        "匿名".to_string()
    } else {
        item.author.clone()
    };
    let count = meta
        .map(|m| m.voteup)
        .filter(|c| *c > 0)
        .unwrap_or(item.voteup);
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
    header.push(Line::from(spans));

    // ④ 正文（只有这段会被滚动）
    let mut body: Vec<Line<'static>> = Vec::new();
    let raw = if item.content.is_empty() {
        item.excerpt.clone()
    } else {
        item.content.clone()
    };
    for l in text::wrap(&raw, w) {
        if l.is_empty() {
            body.push(Line::from(""));
        } else {
            body.push(Line::from(Span::styled(
                format!("  {l}"),
                Style::default().fg(theme::TEXT),
            )));
        }
    }

    (header, body)
}

/// 画吸顶信息条（不滚动）
pub fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    if app.header.is_empty() || area.height == 0 {
        return;
    }
    let lines: Vec<Line<'static>> = app
        .header
        .iter()
        .take(area.height as usize)
        .cloned()
        .collect();
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// 画可滚动的正文
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    if app.lines.is_empty() {
        return;
    }
    let height = area.height as usize;
    // 按页对齐的上限钳制，别把最后一页按回"半屏重叠"的位置
    let max = app.last_page_start();
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
