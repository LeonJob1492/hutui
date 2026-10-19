use super::{AnswerItem, Comment};
use serde_json::Value;

// ---------------------------------------------------------------- JSON 取值

fn as_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn as_i64(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// 把一个 answer / article 实体转成可阅读条目。
/// 推荐流的 `target`、问题回答列表的元素、单篇回答接口的根对象，都是这个形状。
fn entity_to_item(t: &Value) -> Option<AnswerItem> {
    let id = as_str(&t["id"]);
    if id.is_empty() {
        return None;
    }
    let kind = t
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("answer")
        .to_string();

    let content = html_to_text(t.get("content").and_then(|x| x.as_str()).unwrap_or(""));
    let excerpt = html_to_text(t.get("excerpt").and_then(|x| x.as_str()).unwrap_or(""));

    let q = &t["question"];
    let mut question_title = as_str(&q["title"]);
    if question_title.is_empty() {
        question_title = as_str(&t["title"]);
    }

    Some(AnswerItem {
        id,
        kind,
        author: as_str(&t["author"]["name"]),
        voteup: as_i64(&t["voteup_count"]),
        comments: as_i64(&t["comment_count"]),
        content,
        excerpt,
        question_id: as_str(&q["id"]),
        question_title,
    })
}

// ---------------------------------------------------------------- 解析器

/// `/api/v3/feed/topstory/recommend` —— 每页 6 条，`paging.next` 可跟随
pub fn parse_feed_page(v: &Value) -> Vec<AnswerItem> {
    v["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entity_to_item(&entry["target"]))
                .collect()
        })
        .unwrap_or_default()
}

/// `/api/v4/questions/{qid}/answers` —— data 里直接就是 answer 实体
/// 返回 (条目, 总条数, 是否到底)
pub fn parse_answers_page(v: &Value) -> (Vec<AnswerItem>, usize, bool) {
    let items: Vec<AnswerItem> = v["data"]
        .as_array()
        .map(|arr| arr.iter().filter_map(entity_to_item).collect())
        .unwrap_or_default();
    let total = v["paging"]["totals"].as_u64().unwrap_or(items.len() as u64) as usize;
    let is_end = v["paging"]["is_end"].as_bool().unwrap_or(true);
    (items, total, is_end)
}

/// `/api/v4/answers/{aid}` —— 根对象就是 answer 实体
pub fn parse_answer_entity(v: &Value) -> Option<AnswerItem> {
    entity_to_item(v)
}

/// 跟随分页用的下一页地址
pub fn next_page_url(v: &Value) -> Option<String> {
    v["paging"]["next"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------- HTML → 文本

pub fn html_to_text(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();
    let mut tag = String::new();
    let mut in_tag = false;

    while let Some(c) = chars.next() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag.trim().to_ascii_lowercase();
                let name: String = t
                    .trim_start_matches('/')
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric())
                    .collect();
                match name.as_str() {
                    "br" => out.push('\n'),
                    "p" | "div" | "tr" | "figure" | "blockquote" | "h1" | "h2" | "h3" => {
                        out.push('\n')
                    }
                    "li" => {
                        out.push('\n');
                        out.push_str("· ");
                    }
                    "img" => out.push_str("\n[图]\n"),
                    "script" | "style" => skip_block(&mut chars, &name),
                    _ => {}
                }
            } else {
                tag.push(c);
            }
        } else if c == '<' {
            in_tag = true;
            tag.clear();
        } else {
            out.push(c);
        }
    }

    tidy(&decode_entities(&out))
}

fn skip_block<I: Iterator<Item = char>>(it: &mut std::iter::Peekable<I>, name: &str) {
    let close = format!("</{name}");
    let mut window = String::new();
    for c in it.by_ref() {
        window.push(c.to_ascii_lowercase());
        while window.chars().count() > close.chars().count() {
            let skip = window.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            window.drain(..skip);
        }
        if window == close {
            for c2 in it.by_ref() {
                if c2 == '>' {
                    break;
                }
            }
            return;
        }
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…")
        .replace("&middot;", "·")
        .replace("&amp;", "&")
}

fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0usize;
    for line in s.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_feed_item() {
        let v = json!({
            "data": [{
                "type": "feed",
                "target": {
                    "type": "answer", "id": "2093", "voteup_count": 110,
                    "content": "<p>第一段</p><p>第二段</p>",
                    "excerpt": "摘<em>要</em>",
                    "author": {"name": "某人"},
                    "question": {"id": "1945", "title": "问题标题"}
                }
            }]
        });
        let items = parse_feed_page(&v);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "2093");
        assert_eq!(items[0].author, "某人");
        assert_eq!(items[0].question_id, "1945");
        assert!(items[0].content.contains("第一段"));
        assert!(!items[0].content.contains('<'));
    }

    #[test]
    fn parses_answers_page_with_totals() {
        let v = json!({
            "data": [{ "type": "answer", "id": "7", "content": "<b>x</b>",
                       "question": { "id": "1", "title": "t" } }],
            "paging": { "is_end": false, "totals": 15 }
        });
        let (items, total, is_end) = parse_answers_page(&v);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "x");
        assert_eq!(total, 15);
        assert!(!is_end);
    }

    #[test]
    fn drops_script_blocks() {
        let t = html_to_text("<div>hi<script>var a = 1 < 2;</script>there</div>");
        assert!(t.contains("hi"));
        assert!(t.contains("there"));
        assert!(!t.contains("var a"));
    }
}

// ---------------------------------------------------------------- 评论

/// `/api/v4/answers/{aid}/comments` —— 作者在 `author.member.name`
pub fn parse_comments(v: &Value) -> (Vec<Comment>, usize) {
    let items = v["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| Comment {
                    id: as_str(&c["id"]),
                    author: as_str(&c["author"]["member"]["name"]),
                    content: html_to_text(c.get("content").and_then(|x| x.as_str()).unwrap_or("")),
                    voteup: as_i64(&c["vote_count"]),
                    children: as_i64(&c["child_comment_count"]),
                    created: as_i64(&c["created_time"]),
                })
                .collect::<Vec<Comment>>()
        })
        .unwrap_or_default();
    let total = v["paging"]["totals"].as_u64().unwrap_or(items.len() as u64) as usize;
    (items, total)
}

/// 找默认收藏夹：`/api/v4/people/{token}/collections`
pub fn parse_default_collection(v: &Value) -> Option<String> {
    let arr = v["data"].as_array()?;
    let pick = arr
        .iter()
        .find(|c| c["is_default"].as_bool().unwrap_or(false))
        .or_else(|| arr.first())?;
    let id = as_str(&pick["id"]);
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}
