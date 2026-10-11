use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentType {
    Line,
    Block,
    Docstring,
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
