use std::collections::HashSet;

use crate::engine;
use crate::models::{CommentInfo, CommentType};

const SUPPRESSION_MARKER: &str = "comment-checker: allow";

/// Reasons reported for a suppression marker that does not supplant the change it rides in.
const ALLOW_ADDED_REASON: &str = "allow-added";
const ALLOW_INVALID_REASON: &str = "allow-invalid";

/// The spared classes a marker may name as its reason class, one id per spare rule.
const SPARED_CLASSES: &[&str] = &[
    "license-header",
    "generated-file",
    "public-api-doc",
    "attribution",
    "bdd",
    "directive",
    "shebang",
];

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
        content.lines().any(is_line_suppressed)
    }
}

/// Whether the text carries a syntactically valid suppression marker.
#[must_use]
pub fn is_line_suppressed(text: &str) -> bool {
    marker_offset(text).is_some_and(|offset| marker_validity(text, offset) == MarkerValidity::Valid)
}

/// Whether the marker tail names a spared class and a non-empty justification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarkerValidity {
    Valid,
    Invalid,
}

fn marker_offset(text: &str) -> Option<usize> {
    text.to_ascii_lowercase().find(SUPPRESSION_MARKER)
}

/// Classify the marker whose start is `offset` in `text`.
///
/// `comment-checker: allow` is valid on its own; the long form is
/// `comment-checker: allow <reason-class> -- <justification>`, where the class is one of
/// [`SPARED_CLASSES`].
fn marker_validity(text: &str, offset: usize) -> MarkerValidity {
    let rest = trim_comment_terminator(&text[offset + SUPPRESSION_MARKER.len()..]);
    if rest.is_empty() {
        return MarkerValidity::Valid;
    }
    let Some((class, justification)) = rest.split_once("--") else {
        return MarkerValidity::Invalid;
    };
    let class = class.trim();
    let justification = justification.trim();
    if !SPARED_CLASSES
        .iter()
        .any(|known| known.eq_ignore_ascii_case(class))
        || justification.is_empty()
    {
        return MarkerValidity::Invalid;
    }
    MarkerValidity::Valid
}

fn trim_comment_terminator(text: &str) -> &str {
    let text = text.trim();
    for suffix in ["*/", "-->", "*)", "\"\"\"", "'''"] {
        if let Some(head) = text.strip_suffix(suffix) {
            return head.trim();
        }
    }
    text
}

/// Whether the detected item is a comment node rather than a string-literal docstring.
///
/// Python docstrings are string literals, so marker text inside them is text, not a comment. The
/// JavaScript/Java docstrings come from the grammar's comment query and keep comment syntax.
fn is_comment_node(comment: &CommentInfo) -> bool {
    comment.comment_type != CommentType::Docstring || comment.text.trim_start().starts_with("/**")
}

/// The marker-level outcome for one analyzed fragment.
#[derive(Debug, Default)]
pub(crate) struct SuppressionAnalysis {
    /// A valid marker outside an added line suppresses the whole fragment.
    pub(crate) suppressed: bool,
    /// The report reason for a marker comment, aligned with the analyzed comment slice.
    pub(crate) marker_reasons: Vec<Option<&'static str>>,
}

/// Analyze the markers carried by host comment nodes in `content`.
///
/// A marker only counts when it sits inside a comment node's byte span, so the same text inside a
/// string literal does nothing. `added` is the set of 0-based lines this change added; `None` means
/// no line counts as added (a whole-file write, or a fragment with no base text).
pub(crate) fn analyze_suppression(
    content: &str,
    comments: &[CommentInfo],
    added: Option<&HashSet<usize>>,
) -> SuppressionAnalysis {
    let mut analysis = SuppressionAnalysis {
        suppressed: false,
        marker_reasons: vec![None; comments.len()],
    };
    for (index, comment) in comments.iter().enumerate() {
        if !is_comment_node(comment) {
            continue;
        }
        let Some(offset) = marker_offset(&comment.text) else {
            continue;
        };
        match marker_validity(&comment.text, offset) {
            MarkerValidity::Invalid => analysis.marker_reasons[index] = Some(ALLOW_INVALID_REASON),
            MarkerValidity::Valid => {
                let row = marker_row(content, comment, offset);
                if added.is_some_and(|lines| lines.contains(&row)) {
                    analysis.marker_reasons[index] = Some(ALLOW_ADDED_REASON);
                } else {
                    analysis.suppressed = true;
                }
            }
        }
    }
    analysis
}

/// The 0-based host line carrying the marker `offset` bytes into `comment`.
///
/// A host capture's span gives the exact host byte. An injected capture indexes injected text, so
/// its start line plus the newlines inside the comment is the host line.
fn marker_row(content: &str, comment: &CommentInfo, offset: usize) -> usize {
    if let Some((start, end)) = comment.byte_span {
        if start <= end
            && end <= content.len()
            && content.get(start..end) == Some(comment.text.as_str())
        {
            return content[..start + offset].matches('\n').count();
        }
    }
    comment.line_number.saturating_sub(1) + comment.text[..offset].matches('\n').count()
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

/// Punctuation that, on its own, reads as code (arrows, compound operators, braces). `;`, `--` and
/// `==` are deliberately absent: they are common in prose and in directive descriptions (`-- reason`,
/// `; note`), so they cannot prove a line is commented-out code.
const CODE_SHAPE_PUNCTUATION: &[&str] = &["=>", ":=", "++", "{", "}"];

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

/// True when the text is shaped like an assignment: a bare identifier path, then a single `=`
/// (spaced or compact), then a value. A comparison or compound operator (`==`, `!=`, `<=`, `+=`,
/// `:=`, …) is not an assignment, and prose is rejected by the path-shaped left side.
fn looks_like_assignment(text: &str) -> bool {
    let bytes = text.as_bytes();
    let Some(index) = (0..bytes.len()).find(|&index| {
        bytes[index] == b'='
            && bytes.get(index + 1).copied() != Some(b'=')
            && !matches!(
                (index > 0).then(|| bytes[index - 1]),
                Some(
                    b'=' | b'!'
                        | b'<'
                        | b'>'
                        | b'+'
                        | b'-'
                        | b'*'
                        | b'/'
                        | b'%'
                        | b'&'
                        | b'|'
                        | b'^'
                        | b':'
                )
            )
    }) else {
        return false;
    };
    let lhs = text[..index].trim();
    let rhs = text[index + 1..].trim();
    if rhs.is_empty() {
        return false;
    }
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

/// A line that reads as commented-out code: an assignment, an arrow/brace construct, or a statement
/// keyword paired with code punctuation. A keyword or bare punctuation alone is not enough, so a
/// directive description (`-- reason`, `; note`) or prose (`if you need…`) is not mistaken for code.
fn is_code_shaped(stripped: &str) -> bool {
    looks_like_assignment(stripped)
        || any_contains(stripped, CODE_SHAPE_PUNCTUATION)
        || (any_starts(stripped, CODE_KEYWORDS)
            && any_contains(stripped, &[";", "(", ")", "=", ":"]))
}

fn is_commented_out_code(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    is_code_shaped(stripped_after_marker(&text))
}

fn is_vacuous_todo(comment: &CommentInfo) -> bool {
    let text = normalized(comment);
    any_starts(stripped_after_marker(&text), TODO_PREFIXES)
}

/// Phrases that narrate how the code got here instead of stating what it does now. Matched on the
/// lowercased text with one comment marker stripped.
const PAST_FACING_PHRASES: &[&str] = &[
    "unchanged from previous",
    "unchanged from the previous",
    "unchanged from old",
    "unchanged from the old",
    "unchanged from original",
    "unchanged from the original",
    "same as before",
    "same as it was",
    "same as the old",
    "as was",
    "as it was",
    "as used to be",
    "as it used to be",
    "this used to",
    "used to be",
    "formerly",
    "previously was",
    "previously called",
    "previously named",
    "copied from",
    "copied over",
    "kept as is",
    "kept unchanged",
    "kept as it was",
    "left as is",
    "left unchanged",
    "left untouched",
];

const PRESERVE_VERBS: &[&str] = &["preserve", "preserves", "preserved"];
const PRESERVED_BEHAVIORS: &[&str] = &["existing", "previous", "original", "old", "current"];

fn preserves_behavior(text: &str) -> bool {
    PRESERVE_VERBS.iter().any(|verb| {
        PRESERVED_BEHAVIORS.iter().any(|adjective| {
            text.contains(&format!("{verb} {adjective} behavior"))
                || text.contains(&format!("{verb} the {adjective} behavior"))
        })
    })
}

fn is_past_facing(comment: &CommentInfo) -> bool {
    let text = comment.text.trim().to_lowercase();
    let text = stripped_after_marker(&text);
    preserves_behavior(text) || any_contains(text, PAST_FACING_PHRASES)
}

/// Justifications the comment's own structure proves: a license, a generated notice, public-API
/// markup, or provenance. They stay unconditional because no keyword or smell can make them false.
fn is_structurally_justified(comment: &CommentInfo) -> bool {
    is_license(comment)
        || is_generated_file(comment)
        || is_public_api_doc(comment)
        || is_attribution(comment)
}

/// A text-level smell proven by the comment's own body; a marker must not acquit it (D13).
fn is_proven_smell(comment: &CommentInfo) -> bool {
    is_commented_out_code(comment)
        || is_vacuous_todo(comment)
        || AgentMemoFilter.is_agent_memo(comment)
}

/// Environment variable that selects how far the prose-keyword allow-list may acquit. Missing,
/// empty, or any unrecognized value keeps [`KeywordAcquitMode::ProvenOnly`]; that fail-open default
/// means a typo narrows nothing.
const KEYWORD_ACQUIT_ENV: &str = "COMMENT_CHECKER_KEYWORD_ACQUIT";

/// How the prose-keyword fallback acquits a comment. Structural justifications, directives, BDD
/// markers, shebangs, and the opt-out marker spare a comment in both modes.
#[derive(Clone, Copy, Eq, PartialEq)]
enum KeywordAcquitMode {
    /// A prose keyword acquits unless a proven smell or cited evidence outranks it.
    ProvenOnly,
    /// Prose keywords never acquit; a rationale comment is reported on its text-level reason.
    Never,
}

/// Read the mode on each invocation; a process that changes the variable between runs sees the
/// change, and nothing is cached across calls.
fn keyword_acquit_mode() -> KeywordAcquitMode {
    if std::env::var(KEYWORD_ACQUIT_ENV).is_ok_and(|value| value == "never") {
        KeywordAcquitMode::Never
    } else {
        KeywordAcquitMode::ProvenOnly
    }
}

/// A reason that cites adjacent code. Only a whole-file detection supplies it, so it is proof the
/// prose-keyword allow-list cannot overturn.
fn is_contextual_reason(reason: &str) -> bool {
    reason == "narrates-control-flow" || reason.starts_with("restates-code:")
}

fn reason_for(comment: &CommentInfo) -> &'static str {
    if is_commented_out_code(comment) {
        "commented-out-code"
    } else if is_vacuous_todo(comment) {
        "vacuous-todo"
    } else if AgentMemoFilter.is_agent_memo(comment) {
        "agent-memo"
    } else if is_past_facing(comment) {
        "past-facing"
    } else {
        "unnecessary-comment"
    }
}

fn is_spared(comment: &CommentInfo, keyword_acquit: KeywordAcquitMode) -> bool {
    if is_structurally_justified(comment)
        || BddFilter.should_skip(comment)
        || ShebangFilter.should_skip(comment)
    {
        return true;
    }
    // D13: a directive or the opt-out marker must not acquit a proven text smell.
    if is_proven_smell(comment) || is_past_facing(comment) {
        return false;
    }
    if DirectiveFilter.should_skip(comment) || SuppressionFilter.should_skip(comment) {
        return true;
    }
    // The prose keywords are a fallback: cited evidence from adjacent code outranks them, so a
    // narration that says "because" cannot use the word to escape a restates-code verdict. In
    // `never` mode the fallback is gone, so a rationale comment keeps its text-level reason.
    keyword_acquit == KeywordAcquitMode::ProvenOnly
        && is_non_obvious_intent(comment)
        && !comment
            .metadata
            .get("reason")
            .is_some_and(|reason| is_contextual_reason(reason))
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

/// (verb the comment uses, operators the code shows for it). A verb whose operator sits in the
/// adjacent code is a restatement even when no content token overlaps.
const OPERATOR_TABLE: &[(&str, &[&str])] = &[
    ("increment", &["+=", "++"]),
    ("increments", &["+=", "++"]),
    ("incrementing", &["+=", "++"]),
    ("decrement", &["-=", "--"]),
    ("decrements", &["-=", "--"]),
    ("decrementing", &["-=", "--"]),
    ("assign", &["=", ":="]),
    ("assigns", &["=", ":="]),
    ("assigned", &["=", ":="]),
    ("return", &["return"]),
    ("returns", &["return"]),
    ("returning", &["return"]),
    ("add", &["+"]),
    ("adds", &["+"]),
    ("subtract", &["-"]),
    ("subtracts", &["-"]),
    ("multiply", &["*"]),
    ("multiplies", &["*"]),
    ("divide", &["/"]),
    ("divides", &["/"]),
    ("double", &["* 2", "*2"]),
    ("doubles", &["* 2", "*2"]),
    ("halve", &["/ 2", "/2"]),
    ("halves", &["/ 2", "/2"]),
];

/// Blank the payload of `"…"`/`'…'` literals so a citation never quotes literal content. A backslash
/// escape keeps the following character inside the literal, so `"a \" += b"` is fully masked.
fn mask_literals(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in text.chars() {
        if escaped {
            escaped = false;
            output.push(' ');
            continue;
        }
        match quote {
            Some(open) if character == open => {
                quote = None;
                output.push(character);
            }
            Some(_) if character == '\\' => {
                escaped = true;
                output.push(' ');
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
///
/// `start_line` and `end_line` are the one-based inclusive line bounds of the comment, so a
/// multi-line comment never searches inside its own text. (A Python docstring whose stored text is
/// only its first line cannot supply a true end; the bound is then its first line.)
fn adjacent_code(lines: &[&str], start_line: usize, end_line: usize) -> Option<String> {
    let following = lines
        .iter()
        .skip(end_line)
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !is_comment_line(line));
    let preceding = || {
        lines
            .iter()
            .take(start_line.saturating_sub(1))
            .rev()
            .map(|line| line.trim())
            .find(|line| !line.is_empty() && !is_comment_line(line))
    };
    following
        .or_else(preceding)
        .map(|line| line.chars().take(240).collect())
}

/// A verb in the comment whose operator appears in the adjacent code, as `"verb operator"`.
fn operator_evidence(comment_text: &str, adjacent: &str) -> Option<String> {
    let text = stripped_after_marker(comment_text).to_lowercase();
    let words: Vec<&str> = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    for &(verb, operators) in OPERATOR_TABLE {
        if words.contains(&verb) {
            if let Some(operator) = operators
                .iter()
                .copied()
                .find(|operator| adjacent.contains(*operator))
            {
                return Some(format!("{verb} {operator}"));
            }
        }
    }
    None
}

/// A cited reason for a comment that restates or narrates the code beside it, or `None`.
fn restate_reason(comment: &CommentInfo, adjacent: &str) -> Option<String> {
    // Mask literals once, so no sub-check can cite string contents: `log("a += b")` is not a
    // restatement of a comment that says "increment".
    let adjacent = mask_literals(adjacent);
    let comment_tokens = content_tokens(&comment.text);
    if comment_tokens.len() >= 2 {
        let shared = containment(&comment_tokens, &raw_tokens(&adjacent));
        if !shared.is_empty() && shared.len() * 2 >= comment_tokens.len() {
            return Some(format!("restates-code: {}", shared.join(", ")));
        }
    }
    if let Some(evidence) = operator_evidence(&comment.text, &adjacent) {
        return Some(format!("restates-code: {evidence}"));
    }
    if narrates_control_flow(&comment.text, &adjacent) {
        return Some("narrates-control-flow".to_owned());
    }
    None
}

/// Refine a generic reason with cited evidence from adjacent code. `reliable` is false for an
/// Edit/MultiEdit fragment, where the context cannot vouch for a verdict, so nothing is emitted.
///
/// Call this before [`apply_filters`]: a proven retelling must be able to outrank the prose-keyword
/// acquit. A comment already carrying a text-level smell keeps it, so only the generic and
/// past-facing reasons are refined.
pub(crate) fn contextualize(comments: &mut [CommentInfo], content: &str, reliable: bool) {
    if !reliable {
        return;
    }
    let lines: Vec<&str> = content.lines().collect();
    for comment in comments.iter_mut() {
        if is_proven_smell(comment) {
            continue;
        }
        match comment.metadata.get("reason").map(String::as_str) {
            None | Some("unnecessary-comment" | "past-facing") => {}
            Some(_) => continue,
        }
        let start = comment.line_number;
        let end = start + comment.text.matches('\n').count();
        let Some(adjacent) = adjacent_code(&lines, start, end) else {
            continue;
        };
        if let Some(reason) = restate_reason(comment, &adjacent) {
            comment.metadata.insert("reason".to_owned(), reason);
        }
    }
}

#[must_use]
pub fn apply_filters(comments: &[CommentInfo]) -> Vec<CommentInfo> {
    apply_filters_with_markers(comments, &[])
}

/// Like [`apply_filters`], with the marker reasons from [`analyze_suppression`] aligned by index.
///
/// A marker comment is reported with its `allow-added`/`allow-invalid` reason instead of being
/// spared; every other comment goes through the normal spare/flag rules.
#[must_use]
pub(crate) fn apply_filters_with_markers(
    comments: &[CommentInfo],
    marker_reasons: &[Option<&'static str>],
) -> Vec<CommentInfo> {
    let keyword_acquit = keyword_acquit_mode();
    comments
        .iter()
        .enumerate()
        .filter_map(|(index, comment)| {
            if let Some(reason) = marker_reasons.get(index).copied().flatten() {
                let mut kept = comment.clone();
                kept.metadata.insert("reason".to_owned(), reason.to_owned());
                return Some(kept);
            }
            if is_spared(comment, keyword_acquit) {
                return None;
            }
            let mut kept = comment.clone();
            let reason = comment
                .metadata
                .get("reason")
                .filter(|reason| is_contextual_reason(reason))
                .cloned()
                .unwrap_or_else(|| reason_for(comment).to_owned());
            kept.metadata.insert("reason".to_owned(), reason);
            Some(kept)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{INTENT_MARKERS, is_agent_memo_phrase};

    /// Guard: a carried sf intent marker that is also an agent-memo phrase would silently acquit a
    /// memo (the `note:` regression). Keep the two catalogs disjoint.
    #[test]
    fn intent_markers_do_not_overlap_agent_memo_phrases() {
        let overlaps: Vec<&str> = INTENT_MARKERS
            .iter()
            .copied()
            .filter(|marker| is_agent_memo_phrase(marker))
            .collect();
        assert!(
            overlaps.is_empty(),
            "intent markers also read as agent memos: {overlaps:?}"
        );
    }
}
