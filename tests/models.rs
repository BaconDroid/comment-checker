use comment_checker::models::{CommentInfo, CommentType};

#[test]
fn normalized_text_given_whitespace_returns_stripped_lowercase() {
    // given
    let comment = CommentInfo::new("  # HELLO  ", 1, "test.py", CommentType::Line, false);

    // when
    let normalized = comment.normalized_text();

    // then
    assert_eq!(normalized, "# hello");
}
