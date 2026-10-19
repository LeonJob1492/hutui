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

/// CJK 感知换行。ratatui 自带的中西文换行以空格为界，中文整段会被当成一个词，
/// 所以这里自己做：按显示宽度断行，行首禁则字符回退到上一行。
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
                if let Some(pos) = cur.rfind(' ') {
                    let head: String = cur[..pos].to_string();
                    let tail: String = cur[pos + 1..].to_string();
                    if !head.is_empty() {
                        out.push(head);
                        cur = tail;
                        cur_w = width(&cur);
                        if cur_w + cw > max {
                            out.push(std::mem::take(&mut cur));
                            cur_w = 0;
                        }
                    } else {
                        out.push(std::mem::take(&mut cur));
                        cur_w = 0;
                    }
                } else {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
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

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("一二三四五", 6), "一二…");
        assert_eq!(truncate("abc", 10), "abc");
    }
}
