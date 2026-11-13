//! The verdict of a report and the text that is posted.

use agent_core::status::ReviewVerdict;

use super::{MAX_REPORT_CHARS, VerdictProblem, parse_verdict, report_text};

#[test]
fn a_report_that_starts_with_its_verdict_has_that_verdict() {
    assert_eq!(
        parse_verdict("VERDICT: APPROVE\nSUMMARY: fine\nSTATUS: DONE"),
        Ok(ReviewVerdict::Approve)
    );
    assert_eq!(
        parse_verdict("VERDICT: REQUEST_CHANGES\nSUMMARY: not fine\nSTATUS: DONE"),
        Ok(ReviewVerdict::RequestChanges)
    );
}

#[test]
fn what_the_reviewer_said_before_its_report_does_not_matter() {
    let message = "I read the diff and ran the gate.\n\nVERDICT: REQUEST_CHANGES\nSUMMARY: one blocker\nSTATUS: DONE";

    assert_eq!(parse_verdict(message), Ok(ReviewVerdict::RequestChanges));
}

#[test]
fn markup_and_indentation_around_the_verdict_line_are_dropped() {
    for line in [
        "**VERDICT: APPROVE**",
        "**VERDICT:** APPROVE",
        "`VERDICT: APPROVE`",
        "> VERDICT: APPROVE",
        "# VERDICT: APPROVE",
        "- VERDICT: APPROVE",
        "   VERDICT:    APPROVE   ",
        "VERDICT:APPROVE",
    ] {
        assert_eq!(
            parse_verdict(&format!("{line}\nSUMMARY: fine")),
            Ok(ReviewVerdict::Approve),
            "{line}"
        );
    }
}

#[test]
fn the_verdict_may_be_written_in_any_case() {
    assert_eq!(parse_verdict("Verdict: approve"), Ok(ReviewVerdict::Approve));
    assert_eq!(
        parse_verdict("VERDICT: Request_Changes"),
        Ok(ReviewVerdict::RequestChanges)
    );
}

#[test]
fn the_first_verdict_line_decides_and_a_later_one_does_not_overrule_it() {
    let message =
        "VERDICT: REQUEST_CHANGES\nSUMMARY: a blocker\n\nIf it were fixed: VERDICT: APPROVE\nVERDICT: APPROVE";

    assert_eq!(parse_verdict(message), Ok(ReviewVerdict::RequestChanges));
}

#[test]
fn a_report_without_a_verdict_line_has_no_verdict() {
    for message in [
        "",
        "SUMMARY: fine\nSTATUS: DONE",
        "I approve of it.\nSTATUS: DONE",
        "ZVERDICT: APPROVE",
    ] {
        assert_eq!(parse_verdict(message), Err(VerdictProblem::Missing), "{message:?}");
    }
}

#[test]
fn a_verdict_line_with_both_words_or_other_words_is_unreadable_and_says_what_it_found() {
    for (line, found) in [
        ("VERDICT: APPROVE | REQUEST_CHANGES", "APPROVE | REQUEST_CHANGES"),
        ("VERDICT: APPROVE with doubts", "APPROVE with doubts"),
        ("VERDICT: LGTM", "LGTM"),
        ("VERDICT:", ""),
        ("VERDICT: żółć", "żółć"),
    ] {
        assert_eq!(
            parse_verdict(&format!("{line}\nSTATUS: DONE")),
            Err(VerdictProblem::Unreadable {
                found: found.to_owned()
            }),
            "{line}"
        );
    }
}

#[test]
fn an_unreadable_verdict_is_cut_so_that_a_huge_line_does_not_travel() {
    let line = format!("VERDICT: {}", "x".repeat(1000));

    let Err(VerdictProblem::Unreadable { found }) = parse_verdict(&line) else {
        panic!("an unreadable verdict was expected");
    };

    assert_eq!(found.chars().count(), 80);
}

#[test]
fn the_status_line_is_not_part_of_the_report() {
    assert_eq!(
        report_text("VERDICT: APPROVE\nSUMMARY: fine\n\nSTATUS: DONE\n"),
        "VERDICT: APPROVE\nSUMMARY: fine"
    );
    assert_eq!(report_text("VERDICT: APPROVE\n**STATUS: DONE**"), "VERDICT: APPROVE");
}

#[test]
fn a_report_without_a_status_line_is_kept_whole_and_trimmed() {
    assert_eq!(
        report_text("\n  VERDICT: APPROVE\nSUMMARY: fine  \n\n"),
        "VERDICT: APPROVE\nSUMMARY: fine"
    );
    assert_eq!(report_text(""), "");
}

#[test]
fn only_the_last_status_line_is_taken_off() {
    assert_eq!(
        report_text("Do not write STATUS: DONE in the middle.\nVERDICT: APPROVE\nSTATUS: DONE"),
        "Do not write STATUS: DONE in the middle.\nVERDICT: APPROVE"
    );
}

#[test]
fn a_long_report_is_cut_at_a_character() {
    let message = format!("VERDICT: APPROVE\n{}", "ż".repeat(MAX_REPORT_CHARS * 2));

    let report = report_text(&message);

    assert_eq!(report.chars().count(), MAX_REPORT_CHARS);
    assert!(report.starts_with("VERDICT: APPROVE\nżż"));
}
