//! The persisted classification of investigator work.
use super::{ActivityKind, UnknownKeyword};

#[test]
fn activity_kinds_have_stable_texts_and_reject_unknown_values() {
    let texts = ["team-member", "reviewer", "investigator", "task-allocator"];
    for (kind, text) in ActivityKind::ALL.iter().zip(texts) {
        assert_eq!(kind.as_str(), text);
        assert_eq!(text.parse::<ActivityKind>(), Ok(*kind));
        assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(text));
        assert_eq!(
            serde_json::from_value::<ActivityKind>(serde_json::json!(text)).unwrap(),
            *kind
        );
    }
    assert!(
        matches!("réviewer".parse::<ActivityKind>(), Err(UnknownKeyword { what: "activity kind", value, .. }) if value == "réviewer")
    );
    let error = serde_json::from_value::<ActivityKind>(serde_json::json!("review")).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}
