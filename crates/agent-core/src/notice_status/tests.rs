use super::NoticeStatus;

#[test]
fn every_notice_status_has_its_icon_and_label() {
    let cases = [
        (NoticeStatus::Questions, "❓", "waiting for answers"),
        (NoticeStatus::Decision, "❓", "needs your decision"),
        (NoticeStatus::Failed, "❌", "failed"),
        (NoticeStatus::Hung, "⌛", "hung"),
        (NoticeStatus::Paused, "⏸", "paused"),
        (NoticeStatus::Approved, "✅✅", "approved: ready to merge"),
        (NoticeStatus::NotMerged, "❌", "not merged"),
        (NoticeStatus::Closed, "🚫", "pull request closed"),
        (NoticeStatus::NotPublished, "📕", "not published"),
        (NoticeStatus::NoVerdict, "❗", "review without a verdict"),
        (NoticeStatus::Merged, "🟣", "merged"),
        (NoticeStatus::Asleep, "💤", "team asleep"),
        (NoticeStatus::Done, "✅", "done"),
    ];

    for (status, icon, label) in cases {
        assert_eq!(status.icon(), icon, "{status:?} icon");
        assert_eq!(status.label(), label, "{status:?} label");
        assert_eq!(status.to_string(), label, "{status:?} display");
    }
}
