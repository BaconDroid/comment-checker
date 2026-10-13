use crate::engine;
use crate::models::{CommentInfo, CommentType};

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

// --- classifier predicates ----------------------------------------------------------------------
//
// The justification allow-list and the code-smell kind labels below reimplement the classifier of
// `systemfsoftware/comment-checker` (Apache-2.0) — see NOTICE. They are private: the shared parity
// API-shape contract only names `apply_filters`, so the predicates must not become `pub`.

const LICENSE_MARKERS: &[&str] = &[
    "spdx-license-identifier",
    "copyright",
    "licensed under",
    "permission is hereby granted",
    "all rights reserved",
    "redistribution and use",
    "gnu general public license",
    "apache license",
    "mit license",
    "bsd license",
    "mozilla public license",
];

const GENERATED_MARKERS: &[&str] = &[
    "do not edit",
    "auto-generated",
    "autogenerated",
    "code generated",
    "generated by",
];

const INTENT_MARKERS: &[&str] = &[
    "why",
    "because",
    "workaround",
    // "note:" is deliberately absent: this repo's own `AgentMemoFilter` owns that phrase, so
    // sparing it here would silently acquit an agent memo.
    "important:",
    "warning:",
    "caution:",
    "fixes #",
    "issue #",
    "bug #",
    "see http",
    "https://",
    "http://",
    "to avoid",
    "to prevent",
    "must not",
    "should not",
    "must be",
    "deprecated:",
    "algorithm",
    "regex",
    "security",
    "thread-safety",
    "thread safety",
    "1-based",
    "0-based",
    "@link",
];

const ATTRIBUTION_MARKERS: &[&str] = &[
    "@author",
    "@copyright",
    "adapted from",
    "based on",
    "ported from",
    "credit",
    "@see",
    "@link",
];

/// Attribution tags that only count at the start of the comment (after the marker), so prose that
/// merely mentions `ref:`/`source:` mid-sentence is not read as provenance.
const ATTRIBUTION_PREFIXES: &[&str] = &["ref:", "source:"];

const DOC_MARKUP: &[&str] = &[
    "@param",
    "@returns",
    "@return",
    "@throws",
    "@raises",
    "@exception",
    "@example",
    "@see",
    "@author",
    "@deprecated",
    "@since",
    "@type",
    "@typedef",
    "@property",
    "# examples",
    "# panics",
    "# errors",
    "# safety",
    ":param",
    ":return:",
    ":rtype:",
    ":raises:",
    ":type",
    "args:",
    "returns:",
    "raises:",
    "yields:",
    "attributes:",
    "@brief",
    "@details",
];

const CONTRACT_LEAD_EXCLUDED: &[&str] = &["@see", "@author"];
const CONTRACT_LEAD_EXTRAS: &[&str] = &["@note", "@warning"];

const TODO_PREFIXES: &[&str] = &["todo", "fixme"];

const CODE_KEYWORDS: &[&str] = &[
    "if ", "else ", "for ", "while ", "return ", "var ", "let ", "const ", "func ", "fn ",
    "class ", "def ", "import ", "from ", "print", "fmt.", "console.", "package ", "use ", "pub ",
    "throw ", "raise ", "echo ", "require(", "select ", "insert ", "update ", "delete ", "typeof ",
    "async ", "await ", "std::", "self.", "this.",
];

const CODE_PUNCTUATION: &[&str] = &[";", "{", "}", "=>", ":=", "++", "--", "=="];

const COMMENT_MARKERS: &[&str] = &["//", "/*", "#", "--", "*"];

/// Strip one leading comment marker.
fn strip_comment_marker(text: &str) -> &str {
    COMMENT_MARKERS
        .iter()
        .find(|marker| text.starts_with(**marker))
        .map_or(text, |marker| &text[marker.len()..])
}

/// Strip one leading comment marker and any following whitespace.
fn stripped_after_marker(text: &str) -> &str {
    strip_comment_marker(text).trim_start()
}

fn any_contains(text: &str, list: &[&str]) -> bool {
    list.iter().any(|marker| text.contains(marker))
}

fn any_starts(text: &str, list: &[&str]) -> bool {
    list.iter().any(|prefix| text.starts_with(prefix))
}

fn any_starts_after_strip(text: &str, list: &[&str]) -> bool {
    let stripped = stripped_after_marker(text);
    let stripped = stripped.strip_prefix('@').unwrap_or(stripped);
    any_starts(stripped, list)
}

/// True when the first content of the comment (after stripping the marker) is a contract tag.
fn leads_with_contract_markup(text: &str) -> bool {
    let stripped = stripped_after_marker(text);
    if any_starts(stripped, CONTRACT_LEAD_EXCLUDED) {
        return false;
    }
    any_starts(stripped, DOC_MARKUP) || any_starts(stripped, CONTRACT_LEAD_EXTRAS)
}

/// True when the text is shaped like an assignment statement: a bare identifier path, then ` = `,
/// then a value. Deliberately narrow, so prose containing an `=` is not mistaken for code.
fn looks_like_assignment(text: &str) -> bool {
    let Some((lhs, rhs)) = text.split_once(" = ") else {
        return false;
    };
    if rhs.trim().is_empty() {
        return false;
    }
    let lhs = lhs.trim();
    let path_char = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']' | '*' | '&');
    let starts_path = lhs
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || matches!(c, '_' | '*' | '&'));
    !lhs.is_empty() && starts_path && lhs.chars().all(path_char)
}

fn normalized(comment: &CommentInfo) -> String {
    comment.text.trim().to_lowercase()
}

fn is_license(comment: &CommentInfo) -> bool {
    any_contains(&normalized(comment), LICENSE_MARKERS)
}

fn is_generated_file(comment: &CommentInfo) -> bool {
    any_contains(&normalized(comment), GENERATED_MARKERS)
}

fn is_non_obvious_intent(comment: &CommentInfo) -> bool {
    any_contains(&normalized(comment), INTENT_MARKERS)
}

fn is_attribution(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    any_contains(&text, ATTRIBUTION_MARKERS) || any_starts_after_strip(&text, ATTRIBUTION_PREFIXES)
}

/// Public-API documentation. Without structural context (added in a later phase) only a docstring
/// is promoted by markup alone; a line or block comment that merely leads with a contract tag is not.
fn is_public_api_doc(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    let markup = match comment.comment_type {
        CommentType::Docstring => any_contains(&text, DOC_MARKUP),
        CommentType::Line | CommentType::Block => leads_with_contract_markup(&text),
    };
    markup && comment.comment_type == CommentType::Docstring
}

fn is_commented_out_code(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    let stripped = stripped_after_marker(&text);
    any_starts(stripped, CODE_KEYWORDS)
        || any_contains(stripped, CODE_PUNCTUATION)
        || looks_like_assignment(stripped)
}

fn is_vacuous_todo(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    any_starts(stripped_after_marker(&text), TODO_PREFIXES)
}

fn is_justified(comment: &CommentInfo) -> bool {
    is_license(comment)
        || is_generated_file(comment)
        || is_public_api_doc(comment)
        || is_non_obvious_intent(comment)
        || is_attribution(comment)
}

/// A text-level smell proven by the comment's own body; a marker must not acquit it (D13).
fn is_proven_smell(comment: &CommentInfo) -> bool {
    is_commented_out_code(comment)
        || is_vacuous_todo(comment)
        || AgentMemoFilter.is_agent_memo(comment)
}

fn reason_for(comment: &CommentInfo) -> &'static str {
    if is_commented_out_code(comment) {
        "commented-out-code"
    } else if is_vacuous_todo(comment) {
        "vacuous-todo"
    } else if AgentMemoFilter.is_agent_memo(comment) {
        "agent-memo"
    } else {
        "unnecessary-comment"
    }
}

fn is_spared(comment: &CommentInfo) -> bool {
    if is_justified(comment) || BddFilter.should_skip(comment) || ShebangFilter.should_skip(comment)
    {
        return true;
    }
    // D13: a directive or the opt-out marker must not acquit a proven text smell.
    if is_proven_smell(comment) {
        return false;
    }
    DirectiveFilter.should_skip(comment) || SuppressionFilter.should_skip(comment)
}

// --- contextual evidence ------------------------------------------------------------------------
//
// When the parser saw the whole file a comment can be checked against the code next to it, and a
// restatement or a control-flow narration can cite what it repeats. A fragment (Edit/MultiEdit) has
// no trustworthy neighbours, so no citation is emitted there.

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "is", "are", "for", "with", "this", "that",
    "it", "we", "be", "as", "on", "at", "by", "from", "into", "then", "when", "while",
];

const FLOW_VERBS: &[&str] = &[
    "loop",
    "loops",
    "iterate",
    "iterates",
    "iterating",
    "iteration",
];

const LOOP_CONSTRUCTS: &[&str] = &["for ", "for(", "while ", "while(", "foreach"];

/// Blank the payload of `"…"`/`'…'` literals so a citation never quotes literal content.
fn mask_literals(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    for character in text.chars() {
        match quote {
            Some(open) if character == open => {
                quote = None;
                output.push(character);
            }
            Some(_) => output.push(' '),
            None if character == '"' || character == '\'' => {
                quote = Some(character);
                output.push(character);
            }
            None => output.push(character),
        }
    }
    output
}

fn raw_tokens(text: &str) -> Vec<String> {
    mask_literals(text)
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| token.len() >= 3 && !STOP_WORDS.contains(token))
        .map(str::to_owned)
        .collect()
}

/// Tokens of a comment's prose (marker stripped).
fn content_tokens(text: &str) -> Vec<String> {
    raw_tokens(stripped_after_marker(text))
}

/// The comment's tokens that also appear in the adjacent code, in comment order, deduplicated.
fn containment(comment_tokens: &[String], adjacent_tokens: &[String]) -> Vec<String> {
    let mut shared: Vec<String> = Vec::new();
    for token in comment_tokens {
        if adjacent_tokens.iter().any(|candidate| candidate == token)
            && !shared.iter().any(|seen| seen == token)
        {
            shared.push(token.clone());
        }
    }
    shared
}

fn narrates_control_flow(comment_text: &str, adjacent: &str) -> bool {
    let text = stripped_after_marker(comment_text).to_lowercase();
    let opens_with_verb = FLOW_VERBS.iter().any(|verb| text.starts_with(verb));
    opens_with_verb
        && LOOP_CONSTRUCTS
            .iter()
            .any(|construct| adjacent.to_lowercase().contains(construct))
}

fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    COMMENT_MARKERS
        .iter()
        .any(|marker| trimmed.starts_with(marker))
}

/// The nearest following code line, else the nearest preceding one, capped at 240 bytes.
fn adjacent_code(lines: &[&str], line_number: usize) -> Option<String> {
    let following = lines
        .iter()
        .skip(line_number)
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !is_comment_line(line));
    let preceding = || {
        lines
            .iter()
            .take(line_number.saturating_sub(1))
            .rev()
            .map(|line| line.trim())
            .find(|line| !line.is_empty() && !is_comment_line(line))
    };
    following
        .or_else(preceding)
        .map(|line| line.chars().take(240).collect())
}

/// A cited reason for a comment that restates or narrates the code beside it, or `None`.
fn restate_reason(comment: &CommentInfo, adjacent: &str) -> Option<String> {
    let comment_tokens = content_tokens(&comment.text);
    if comment_tokens.len() >= 2 {
        let shared = containment(&comment_tokens, &raw_tokens(adjacent));
        if !shared.is_empty() && shared.len() * 2 >= comment_tokens.len() {
            return Some(format!("restates-code: {}", shared.join(", ")));
        }
    }
    if narrates_control_flow(&comment.text, adjacent) {
        return Some("narrates-control-flow".to_owned());
    }
    None
}

/// Refine a generic reason with cited evidence from adjacent code. `reliable` is false for an
/// Edit/MultiEdit fragment, where the context cannot vouch for a verdict, so nothing is emitted.
pub(crate) fn contextualize(comments: &mut [CommentInfo], content: &str, reliable: bool) {
    if !reliable {
        return;
    }
    let lines: Vec<&str> = content.lines().collect();
    for comment in comments.iter_mut() {
        if comment.metadata.get("reason").map(String::as_str) != Some("unnecessary-comment") {
            continue;
        }
        let Some(adjacent) = adjacent_code(&lines, comment.line_number) else {
            continue;
        };
        if let Some(reason) = restate_reason(comment, &adjacent) {
            comment.metadata.insert("reason".to_owned(), reason);
        }
    }
}

#[must_use]
pub fn apply_filters(comments: &[CommentInfo]) -> Vec<CommentInfo> {
    comments
        .iter()
        .filter(|comment| !is_spared(comment))
        .map(|comment| {
            let mut kept = comment.clone();
            kept.metadata
                .insert("reason".to_owned(), reason_for(comment).to_owned());
            kept
        })
        .collect()
}
