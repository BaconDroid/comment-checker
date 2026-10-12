use crate::engine;
use crate::models::CommentInfo;

const SUPPRESSION_MARKER: &str = "comment-checker: allow";

#[derive(Clone, Copy, Debug, Default)]
pub struct BddFilter;

impl BddFilter {
    #[must_use]
    pub fn should_skip(&self, comment: &CommentInfo) -> bool {
        let normalized = engine::strip_one_prefix(comment.text.trim(), &["#", "//", "--"]);
        engine::BDD_KEYWORDS
            .iter()
            .any(|keyword| normalized.eq_ignore_ascii_case(keyword))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectiveFilter;

impl DirectiveFilter {
    #[must_use]
    pub fn should_skip(&self, comment: &CommentInfo) -> bool {
        let mut normalized =
            engine::strip_one_prefix(comment.text.trim(), &["#", "//", "/*", "--"]);
        if let Some(rest) = normalized.strip_prefix('@') {
            normalized = rest.trim();
        }
        engine::DIRECTIVE_PREFIXES
            .iter()
            .any(|directive| engine::starts_with_ignore_ascii_case(normalized, directive))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ShebangFilter;

impl ShebangFilter {
    #[must_use]
    pub fn should_skip(&self, comment: &CommentInfo) -> bool {
        comment.text.trim().starts_with("#!")
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SuppressionFilter;

impl SuppressionFilter {
    #[must_use]
    pub fn should_skip(&self, comment: &CommentInfo) -> bool {
        is_line_suppressed(&comment.text)
    }

    #[must_use]
    pub fn is_file_suppressed(&self, content: &str) -> bool {
        engine::is_file_suppressed(content, SUPPRESSION_MARKER)
    }
}

#[must_use]
pub fn is_line_suppressed(text: &str) -> bool {
    engine::is_line_suppressed(text, SUPPRESSION_MARKER)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AgentMemoFilter;

impl AgentMemoFilter {
    #[must_use]
    pub fn is_agent_memo(&self, comment: &CommentInfo) -> bool {
        let text = strip_agent_prefixes(comment.text.trim());
        is_agent_memo_phrase(text)
    }
}

fn strip_agent_prefixes(mut text: &str) -> &str {
    for prefix in ["#", "//", "/*", "--", "*"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim();
        }
    }
    text
}

/// The agent-memo phrase catalog, applied to comment text.
#[must_use]
pub fn is_agent_memo_phrase(text: &str) -> bool {
    let starts_like_memo = match engine::first_ascii_lowercase_byte(text) {
        Some(b'a') => starts_with_any(text, &["added", "add", "after this"]),
        Some(b'b') => engine::starts_with_ignore_ascii_case(text, "before this"),
        Some(b'c') => {
            engine::starts_with_word_pair(text, "changed", &["from", "to"])
                || engine::starts_with_word_pair(text, "change", &["from", "to"])
                || engine::starts_with_word_pair(text, "converted", &["from", "to"])
                || engine::starts_with_word_pair(text, "convert", &["from", "to"])
        }
        Some(b'd') => starts_with_any(text, &["deleted", "delete"]),
        Some(b'h') => engine::starts_with_ignore_ascii_case(text, "here we"),
        Some(b'i') => {
            starts_with_any(text, &["implemented", "implement"])
                || engine::starts_with_word_pair(text, "implementation", &["of", "note"])
        }
        Some(b'm') => {
            engine::starts_with_word_pair(text, "modified", &["from", "to"])
                || engine::starts_with_word_pair(text, "modify", &["from", "to"])
                || engine::starts_with_word_pair(text, "moved", &["from", "to"])
                || engine::starts_with_word_pair(text, "move", &["from", "to"])
                || engine::starts_with_word_pair(text, "migrated", &["from", "to"])
                || engine::starts_with_word_pair(text, "migrate", &["from", "to"])
        }
        Some(b'n') => {
            engine::starts_with_word_pair(text, "now", &["we", "this", "it"])
                || engine::starts_with_ignore_ascii_case(text, "note:")
        }
        Some(b'p') => engine::starts_with_ignore_ascii_case(text, "previously"),
        Some(b'r') => {
            starts_with_any(
                text,
                &["refactor", "replaced", "replace", "removed", "remove"],
            ) || engine::starts_with_word_pair(text, "renamed", &["from", "to"])
                || engine::starts_with_word_pair(text, "rename", &["from", "to"])
        }
        Some(b's') => {
            engine::starts_with_word_pair(text, "switched", &["from", "to"])
                || engine::starts_with_word_pair(text, "switch", &["from", "to"])
        }
        Some(b't') => engine::starts_with_word_pair(
            text,
            "this",
            &[
                "implements",
                "implement",
                "adds",
                "add",
                "removes",
                "remove",
                "changes",
                "change",
                "fixes",
                "fix",
            ],
        ),
        Some(b'u') => {
            engine::starts_with_word_pair(text, "updated", &["from", "to"])
                || engine::starts_with_word_pair(text, "update", &["from", "to"])
        }
        Some(b'w') => engine::starts_with_word_pair(text, "was", &["changed"]),
        _ => false,
    };
    starts_like_memo || engine::has_ascii_arrow(text)
}

fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| engine::starts_with_ignore_ascii_case(text, prefix))
}

#[must_use]
pub fn apply_filters(comments: &[CommentInfo]) -> Vec<CommentInfo> {
    let bdd = BddFilter;
    let directive = DirectiveFilter;
    let shebang = ShebangFilter;
    let suppression = SuppressionFilter;
    comments
        .iter()
        .filter(|comment| {
            !bdd.should_skip(comment)
                && !directive.should_skip(comment)
                && !shebang.should_skip(comment)
                && !suppression.should_skip(comment)
        })
        .cloned()
        .collect()
}
