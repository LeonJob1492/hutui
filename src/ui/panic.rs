use crate::ui::theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

fn prompt() -> Line<'static> {
    Line::from(vec![
        Span::styled("~/Code/hutui", Style::default().fg(theme::ACCENT)),
        Span::styled(" main* ", Style::default().fg(theme::DIM)),
        Span::styled("❯ ", Style::default().fg(theme::OK)),
    ])
}

/// panic 画面：静止的一屏 `cargo test` 输出。
///
/// 刻意零动效 —— 会滚动的画面反而吸引注意，静止的终端是自然的；
/// 而且静止 = 零半帧风险，与「整体重绘不能有异步半帧」的硬约束天然契合。
pub fn build(_width: u16) -> Vec<Line<'static>> {
    let dim = Style::default().fg(theme::DIM);
    let faint = Style::default().fg(theme::FAINT);
    let text = Style::default().fg(theme::TEXT);
    let ok = Style::default().fg(theme::OK);
    let green = Style::default().fg(theme::OK);

    let mut out: Vec<Line<'static>> = Vec::new();

    out.push(Line::from(vec![
        Span::styled("~/Code/hutui", Style::default().fg(theme::ACCENT)),
        Span::styled(" main* ", dim),
        Span::styled("❯ ", Style::default().fg(theme::OK)),
        Span::styled("cargo test --all-features", text),
    ]));
    out.push(Line::from(Span::styled(
        "    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.41s",
        green,
    )));
    out.push(Line::from(Span::styled(
        "     Running unittests src/main.rs (target/debug/deps/hutui-4f2a91c3b7d8e0a1)",
        green,
    )));
    out.push(Line::from(""));
    out.push(Line::from(Span::styled("running 14 tests", text)));

    let cases: [&str; 14] = [
        "text::tests::wraps_cjk_by_display_width",
        "text::tests::keeps_punctuation_off_line_start",
        "text::tests::truncates_with_ellipsis",
        "text::tests::compact_num_rounds_to_k",
        "zhihu::parse::tests::parses_feed_item",
        "zhihu::parse::tests::parses_question_feed_card",
        "zhihu::parse::tests::drops_script_blocks",
        "bridge::tests::discovers_debug_port",
        "bridge::tests::reassembles_flattened_session",
        "bridge::tests::captures_without_requesting",
        "app::tests::cursor_survives_empty_feed",
        "app::tests::reading_scroll_clamps_at_end",
        "ui::tests::header_fits_narrow_terminal",
        "ui::tests::footer_truncates_status",
    ];
    for name in cases {
        out.push(Line::from(vec![
            Span::styled(format!("test {name} "), text),
            Span::styled("... ", faint),
            Span::styled("ok", ok),
        ]));
    }

    out.push(Line::from(""));
    out.push(Line::from(Span::styled(
        "test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s",
        ok,
    )));
    out.push(Line::from(""));
    out.push(Line::from(Span::styled("   Doc-tests hutui", text)));
    out.push(Line::from(""));
    out.push(Line::from(Span::styled("running 0 tests", text)));
    out.push(Line::from(""));
    out.push(Line::from(Span::styled(
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s",
        ok,
    )));
    out.push(Line::from(""));
    out.push(prompt());

    out
}
