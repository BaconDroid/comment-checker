use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::engine::null_default;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentType {
    Line,
    Block,
    Docstring,
}

/// How strongly a reported reason indicts the comment.
///
/// The tier follows the reason class: proven content smells read high, adjacency-dependent
/// contextual reasons read medium because their evidence needs a whole-file `Write`, and the
/// residual prose reason reads low.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    High,
    Medium,
    Low,
}

/// How much `--strip` may remove on its own, shared with test-checker.
///
/// `conservative` removes only proven structural problems, `balanced` adds the audited
/// heuristics, and `aggressive` removes everything the detector flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StripPolicy {
    Conservative,
    Balanced,
    Aggressive,
}

impl StripPolicy {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "conservative" => Some(Self::Conservative),
            "balanced" => Some(Self::Balanced),
            "aggressive" => Some(Self::Aggressive),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Conservative => "conservative",
            Self::Balanced => "balanced",
            Self::Aggressive => "aggressive",
        }
    }

    /// Whether a reason at this severity may be removed on its own.
    ///
    /// comment-checker's residual prose (`unnecessary-comment`) is a low-tier reason but the
    /// normal cleanup target, so `balanced` covers every tier and `conservative` is the opt-out
    /// for a tree where only proven smells should disappear.
    #[must_use]
    pub(crate) const fn allows(self, severity: Severity) -> bool {
        match self {
            Self::Aggressive | Self::Balanced => true,
            Self::Conservative => matches!(severity, Severity::High),
        }
    }
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Edit {
    #[serde(default, alias = "oldString", deserialize_with = "null_default")]
    pub old_string: String,
    #[serde(default, alias = "newString", deserialize_with = "null_default")]
    pub new_string: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentInfo {
    pub text: String,
    pub line_number: usize,
    pub file_path: String,
    pub comment_type: CommentType,
    pub is_docstring: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    // Off the wire because detect_comments_json is a stable public shape; the removal pass reads
    // it to edit the exact node. None when the span is relative to injected text, not the host.
    #[serde(skip)]
    pub(crate) byte_span: Option<(usize, usize)>,
}

impl CommentInfo {
    #[must_use]
    pub fn new(
        text: impl Into<String>,
        line_number: usize,
        file_path: impl Into<String>,
        comment_type: CommentType,
        is_docstring: bool,
    ) -> Self {
        Self {
            text: text.into(),
            line_number,
            file_path: file_path.into(),
            comment_type,
            is_docstring,
            metadata: BTreeMap::new(),
            byte_span: None,
        }
    }

    #[must_use]
    pub fn normalized_text(&self) -> String {
        self.text.trim().to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::{CommentInfo, CommentType};

    #[test]
    fn comment_info_round_trips_through_json() -> serde_json::Result<()> {
        // given
        let comment = CommentInfo::new("# hello", 3, "test.py", CommentType::Line, false);

        // when
        let encoded = serde_json::to_string(&comment)?;
        let decoded: CommentInfo = serde_json::from_str(&encoded)?;

        // then
        assert_eq!(decoded, comment);
        Ok(())
    }
}
