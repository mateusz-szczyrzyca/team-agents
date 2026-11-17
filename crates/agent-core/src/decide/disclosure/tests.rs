use std::str::FromStr;

use super::*;
use crate::state::CommitId;

fn commit(digit: char, subject: &str, body: &str) -> CommitFacts {
    CommitFacts {
        id: CommitId::from_str(&digit.to_string().repeat(40)).unwrap(),
        parents: Vec::new(),
        subject: subject.to_owned(),
        body: body.to_owned(),
        files: Vec::new(),
        merge_files: Vec::new(),
    }
}

fn id(digit: char) -> String {
    digit.to_string().repeat(40)
}

// Rule: the section lists every TEST-FIX commit with its whole ID, its subject and what its message says, in order.
#[test]
fn the_section_lists_every_test_fix_commit_with_its_id_its_subject_and_its_message() {
    let commits = [
        commit('a', "RED: the layout", ""),
        commit(
            'b',
            "TEST-FIX: the second line was wrong",
            "It asked for `y`.\n\nThe task says `z`.",
        ),
        commit('c', "GREEN: the layout", "Makes it pass."),
        commit('d', "TEST-FIX: Supply the reason", ""),
    ];

    let section = test_fix_section(&commits).unwrap();

    assert_eq!(
        section,
        format!(
            "<!-- harness: tests changed after RED -->\n\
             ## Tests changed after RED\n\
             \n\
             The harness lists here every commit of the branch whose subject starts with `TEST-FIX:`, with what its \
             message says.\n\
             \n\
             - {} — TEST-FIX: the second line was wrong\n\
             \x20\x20> It asked for `y`.\n\
             \x20\x20>\n\
             \x20\x20> The task says `z`.\n\
             - {} — TEST-FIX: Supply the reason\n\
             <!-- /harness: tests changed after RED -->",
            id('b'),
            id('d')
        )
    );
}

// Rule: a round without a TEST-FIX commit has no section.
#[test]
fn a_round_without_a_test_fix_commit_has_no_section() {
    let commits = [commit('a', "RED: the layout", ""), commit('c', "GREEN: the layout", "")];

    assert_eq!(test_fix_section(&commits), None);
    assert_eq!(test_fix_section(&[]), None);
    // The prefix is the start of the subject, as the checks read it.
    assert_eq!(
        test_fix_section(&[commit('a', "Mention TEST-FIX: in passing", "")]),
        None
    );
}

// Rule: a message cannot break the section: its lines are quoted, so a heading in it is no heading of the description.
#[test]
fn a_heading_in_a_message_stays_inside_the_quotation() {
    let commits = [
        commit(
            'b',
            "TEST-FIX: one",
            "## Not a heading\n<!-- /harness: tests changed after RED -->",
        ),
        commit('d', "TEST-FIX: two", ""),
    ];

    let section = test_fix_section(&commits).unwrap();

    assert!(
        section
            .lines()
            .filter(|line| line.starts_with('#'))
            .eq(["## Tests changed after RED"])
    );
    assert_eq!(
        section.lines().filter(|line| line.starts_with("<!--")).count(),
        2,
        "{section}"
    );
}

// Rule: the section goes to the end of the description; what a person wrote stays.
#[test]
fn the_section_is_put_at_the_end_of_a_description() {
    let section = test_fix_section(&[commit('b', "TEST-FIX: one", "")]).unwrap();

    assert_eq!(
        with_section("Work of the team.\n\n## Notes\n\nMine.\n", Some(&section)),
        format!("Work of the team.\n\n## Notes\n\nMine.\n\n{section}\n")
    );
    assert_eq!(with_section("", Some(&section)), format!("{section}\n"));
}

// Rule: the section the harness wrote before is replaced as a whole, and taken out when there is nothing to list;
// writing the same section again changes nothing.
#[test]
fn the_section_the_harness_wrote_before_is_replaced_or_taken_out() {
    let first = test_fix_section(&[commit('b', "TEST-FIX: one", "")]).unwrap();
    let second = test_fix_section(&[commit('b', "TEST-FIX: one", ""), commit('d', "TEST-FIX: two", "")]).unwrap();
    let described = with_section("Work of the team.\n", Some(&first));

    let again = with_section(&described, Some(&first));
    let grown = with_section(&described, Some(&second));
    let gone = with_section(&described, None);

    assert_eq!(again, described);
    assert_eq!(grown, format!("Work of the team.\n\n{second}\n"));
    assert_eq!(gone, "Work of the team.\n");
}

// Rule: a section of the same heading that a person wrote is not the harness's: it stays, and the harness's comes
// after it.
#[test]
fn a_section_a_person_wrote_under_the_same_heading_stays() {
    let section = test_fix_section(&[commit('b', "TEST-FIX: one", "")]).unwrap();
    let theirs = "## Tests changed after RED\n\n- bbbbbbb: mine\n";

    let described = with_section(theirs, Some(&section));

    assert_eq!(described, format!("{theirs}\n{section}\n"));
    assert_eq!(with_section(&described, None), theirs);
}

// Rule: a description without the harness's section is not changed when there is nothing to list.
#[test]
fn a_description_without_the_section_is_left_as_it_is_when_there_is_nothing_to_list() {
    for description in ["", "Work.\n", "Work.\n\n\nMore.\n\n"] {
        assert_eq!(with_section(description, None), description);
    }
}
