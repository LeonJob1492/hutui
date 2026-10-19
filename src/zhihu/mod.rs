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
    /// 我对这条的表态，来自 relationship.voting
    pub my_vote: Vote,
    /// 发布时间（unix 秒）
    pub created: i64,
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

/// 一条评论
#[derive(Debug, Clone, Default)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub content: String,
    pub voteup: i64,
    pub children: i64,
    pub created: i64,
}

/// 我对某条内容的表态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vote {
    #[default]
    None,
    Up,
    Down,
}

impl Vote {
    pub fn from_api(s: &str) -> Self {
        match s {
            "up" => Vote::Up,
            "down" => Vote::Down,
            _ => Vote::None,
        }
    }

    pub fn as_api(self) -> &'static str {
        match self {
            Vote::Up => "up",
            Vote::Down => "down",
            Vote::None => "neutral",
        }
    }
}

/// 我对这条内容的本地表态 + 最新计数
#[derive(Debug, Clone, Default)]
pub struct AnswerMeta {
    pub vote: Vote,
    pub voteup: i64,
    pub faved: bool,
}
