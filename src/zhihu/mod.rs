pub mod parse;

/// 一条可阅读内容（回答 / 文章 / 想法）。正文已转成纯文本。
#[derive(Debug, Clone, Default)]
pub struct AnswerItem {
    pub id: String,
    /// answer | article | pin …
    pub kind: String,
    pub author: String,
    pub voteup: i64,
    pub comments: i64,
    pub content: String,
    pub excerpt: String,
    pub question_id: String,
    pub question_title: String,
}

impl AnswerItem {
    /// 列表主标题：优先问题标题，退化为摘要首句
    pub fn headline(&self) -> &str {
        if !self.question_title.is_empty() {
            &self.question_title
        } else {
            &self.excerpt
        }
    }
}
