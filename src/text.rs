use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 终端显示宽度（CJK 记 2）
pub fn width(s: &str) -> usize {
    s.width()
}

/// 不能出现在行首的标点（简易禁则）
const NO_LINE_START: &[char] = &[
    '，', '。', '、', '；', '：', '）', '」', '』', '】', '！', '？', '…', '·', '～', '》', '〉',
    ',', '.', ';', ':', ')', ']', '}', '!', '?', '%',
];

/// 空格软断的最小饱和度（分子/分母）：断点前的部分至少要占满 2/3 行宽。
///
/// 中文段落里空格极少，往往只夹在英文词两侧。若不加这条限制，一个 11 列的
/// 英文词就能把整行从中间劈开——正文里出现「Transformer」独占一行就是这么来的。
const SOFT_BREAK_NUM: usize = 2;
const SOFT_BREAK_DEN: usize = 3;

/// 行尾最后一个 ASCII 词的起始字节位置；行尾不是 ASCII 词则为 None
fn trailing_word_start(s: &str) -> Option<usize> {
    let mut start = None;
    for (i, c) in s.char_indices().rev() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\'') {
            start = Some(i);
        } else {
            break;
        }
    }
    start
}

/// CJK 感知换行。ratatui 自带的中西文换行以空格为界，中文整段会被当成一个词，
/// 所以这里自己做：按显示宽度断行，行首禁则字符回退到上一行。
///
/// 溢出时按优先级选断点：
/// 1. 空格，但断点前必须够满（`SOFT_BREAK_NUM/DEN`），否则中文行会被切得很碎；
/// 2. 硬断，若行尾正卡着半个 ASCII 词，就把整词挪到下一行，不切开单词；
/// 3. 实在放不下（单个超长词）才从中间断。
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(4);
    let mut out: Vec<String> = Vec::new();

    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0usize;

        for raw in para.chars() {
            let c = if raw == '\t' { ' ' } else { raw };
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);

            if cur_w + cw > max && !cur.is_empty() {
                if NO_LINE_START.contains(&c) {
                    // 行首禁则：允许轻微溢出，跟到上一行
                    cur.push(c);
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                    continue;
                }

                // 1) 空格软断，前提是断点前够满
                let mut broken = false;
                if let Some(pos) = cur.rfind(' ') {
                    let head: String = cur[..pos].to_string();
                    if !head.is_empty() && width(&head) * SOFT_BREAK_DEN >= max * SOFT_BREAK_NUM {
                        let tail: String = cur[pos + 1..].to_string();
                        out.push(head);
                        cur = tail;
                        cur_w = width(&cur);
                        if cur_w + cw > max {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                        broken = true;
                    }
                }

                // 2) 硬断；行尾卡着半个 ASCII 词时整词下移
                if !broken {
                    let word_start = if c.is_ascii_alphanumeric() {
                        trailing_word_start(&cur).filter(|&i| i > 0 && width(&cur[i..]) <= max)
                    } else {
                        None
                    };
                    match word_start {
                        Some(i) => {
                            let head = cur[..i].trim_end().to_string();
                            let word = cur[i..].to_string();
                            if head.is_empty() {
                                // 整行就是一个还放不下的长词，只能从中间断
                                out.push(std::mem::take(&mut cur));
                                cur_w = 0;
                            } else {
                                out.push(head);
                                cur = word;
                                cur_w = width(&cur);
                            }
                        }
                        None => {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                    }
                }
            }

            cur.push(c);
            cur_w += cw;
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// 按显示宽度截断并加省略号
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut w = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// 压平换行，用于单行摘要
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 绝对时间：unix 秒 → 「2026-03-02 14:22」（本地时区）
pub fn datetime(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => String::new(),
    }
}

/// 相对时间：1792299923 这样的 unix 秒 → 「3 小时前」
pub fn ago(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let d = (now - ts).max(0);
    if d < 60 {
        "刚刚".to_string()
    } else if d < 3600 {
        format!("{} 分钟前", d / 60)
    } else if d < 86_400 {
        format!("{} 小时前", d / 3600)
    } else if d < 86_400 * 30 {
        format!("{} 天前", d / 86_400)
    } else if d < 86_400 * 365 {
        format!("{} 个月前", (d / (86_400 * 30)).max(1))
    } else {
        format!("{} 年前", (d / (86_400 * 365)).max(1))
    }
}

/// 1100 → 1.1k
pub fn compact_num(n: i64) -> String {
    if n >= 10_000 {
        format!("{:.1}w", n as f64 / 10_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_cjk_by_display_width() {
        let lines = wrap("一二三四五六七八九十", 10);
        assert_eq!(lines, vec!["一二三四五".to_string(), "六七八九十".to_string()]);
    }

    #[test]
    fn keeps_punctuation_off_line_start() {
        let lines = wrap("一二三四五，六七", 10);
        assert_eq!(lines[0], "一二三四五，");
    }

    /// 回归：中文行里夹一个英文词时，不能因为这个空格把行从中间劈开，
    /// 把「Transformer」单独挤成一行（reading 截图里的那个断行错位）。
    #[test]
    fn does_not_orphan_latin_word_in_cjk_line() {
        let src = "很多人对 Transformer 的最大误解，是觉得它在模拟人类某种高级的认知机制。";
        let lines = wrap(src, 30);
        assert!(
            lines.iter().all(|l| l.trim() != "Transformer"),
            "英文词被单独挤成一行: {lines:?}"
        );
        let body = &lines[..lines.len() - 1];
        assert!(
            body.iter().all(|l| width(l) >= 20),
            "除末行外每行都该接近满宽: {lines:?}"
        );
    }

    /// 英文按空格断行时，拼回去必须和原文一致（不吞字、不多空格）
    #[test]
    fn wraps_english_on_spaces() {
        let src = "the quick brown fox jumps over the lazy dog and keeps running on";
        let lines = wrap(src, 30);
        assert!(lines.iter().all(|l| width(l) <= 30));
        assert_eq!(lines.join(" "), src);
    }

    /// 换行不能把 ASCII 单词切成两半
    #[test]
    fn does_not_split_ascii_word() {
        let lines = wrap("中文中文中文中文 Supercalifragilistic 结尾", 20);
        for l in &lines {
            if let Some(i) = l.find("Sup") {
                assert_eq!(&l[i..], "Supercalifragilistic", "单词被切开: {lines:?}");
            }
        }
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("一二三四五", 6), "一二…");
        assert_eq!(truncate("abc", 10), "abc");
    }
}
