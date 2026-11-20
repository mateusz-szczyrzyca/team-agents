use super::*;

use crate::fixtures::at;
use agent_core::questions::{Entry, Verdict};
use agent_core::state::ItemPhase;

fn file(answer: &str) -> QuestionsFile {
    let mut file = QuestionsFile::new(
        "Questions",
        vec![Entry::new(std::num::NonZeroU32::MIN, "Split?").unwrap()],
    )
    .unwrap();
    file.set_verdict(std::num::NonZeroU32::MIN, Verdict::from_text(answer).unwrap())
        .unwrap();
    file
}

fn working() -> Team {
    let mut team = crate::fixtures::team_state().teams()[0].clone();
    let parent = "1.1".parse().unwrap();
    team.open_round("round-20261111-153001-r7q2".parse().unwrap(), Some(parent), at(1))
        .unwrap();
    team.advance_item(
        parent,
        ItemPhase::InProgress {
            assignee: "claude-1".parse().unwrap(),
            branch: "agents/claude-1/first".parse().unwrap(),
        },
    )
    .unwrap();
    team
}

const GOOD: &str = "split\n```json\n{\"parent\":\"1.1\",\"parts\":[{\"title\":\"Résumé\",\"delivers\":\"A document\",\"checks\":\"Readable\",\"depends_on\":[]},{\"title\":\"Review\",\"delivers\":\"A signed review\",\"checks\":\"Accepted\",\"depends_on\":[\"a\"]}]}\n```";

#[test]
fn a_coordinators_complete_form_is_prepared_whole_with_the_parts_named() {
    let team = working();
    let decision = prepare(&team, Role::Coordinator, &file(GOOD));
    assert_eq!(decision.plan.as_ref().map(|plan| plan.parts().len()), Some(2));
    assert_eq!(
        decision.acknowledgement.as_deref(),
        Some("Recorded split parts: 1.1a — Résumé; 1.1b — Review.")
    );
    assert_eq!(team.splits(), &[]);
}

#[test]
fn free_split_words_faulty_forms_and_member_approvals_record_nothing_and_name_why() {
    let team = working();
    for (role, answer, reason) in [
        (
            Role::Coordinator,
            "Please split this into two",
            "use the explicit split verdict form",
        ),
        (
            Role::Coordinator,
            "split\n```json\n{}\n```",
            "invalid split verdict json",
        ),
        (Role::Coordinator, "split\n```yaml\n{}\n```", "requires split followed"),
        (Role::Coordinator, GOOD, "cannot activate"),
        (Role::Member, GOOD, "only the coordinator"),
    ] {
        let target = if reason == "cannot activate" {
            crate::fixtures::team_state().teams()[0].clone()
        } else {
            team.clone()
        };
        let decision = prepare(&target, role, &file(answer));
        assert_eq!(decision.plan, None);
        assert_eq!(
            decision.acknowledgement.as_ref().map(|text| text.contains(reason)),
            Some(true)
        );
        assert_eq!(target.splits(), &[]);
    }
    let ordinary = prepare(&team, Role::Coordinator, &file("ok"));
    assert_eq!(ordinary.plan, None);
    assert_eq!(ordinary.acknowledgement, None);
}

#[test]
fn multiple_split_forms_in_one_file_are_refused_whole() {
    let mut questions = QuestionsFile::new(
        "Questions",
        vec![
            Entry::new(std::num::NonZeroU32::MIN, "First split?").unwrap(),
            Entry::new(std::num::NonZeroU32::new(2).unwrap(), "Second split?").unwrap(),
        ],
    )
    .unwrap();
    for number in [1, 2] {
        questions
            .set_verdict(
                std::num::NonZeroU32::new(number).unwrap(),
                Verdict::from_text(GOOD).unwrap(),
            )
            .unwrap();
    }
    let decision = prepare(&working(), Role::Coordinator, &questions);
    assert_eq!(decision.plan, None);
    assert_eq!(
        decision.acknowledgement.as_deref(),
        Some("No split recorded: approve exactly one split in one questions file.")
    );
}
