//! Exact catalog explanations for every split-policy fault.
use super::*;
use agent_core::decide::split::approval::ProposalFault;
use agent_core::state::split::proposal::Estimate;

#[tokio::test]
async fn every_fault_has_its_own_catalog_text_and_names_its_part() {
    let texts = agent_runtime::team_prompts::TeamPrompts::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &agent_core::config::Language::default(),
    )
    .await
    .unwrap();
    let a = "a".to_owned().try_into().unwrap();
    let b = "b".to_owned().try_into().unwrap();
    let cases = [
        (
            ProposalFault::Unreadable { part: Some(b) },
            "Part b cannot be read in the proposal form. Correct its fields and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::Unreadable { part: None },
            "Part unknown cannot be read in the proposal form. Correct its fields and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::MissingChecks { part: a },
            "Part a has no checks. State how its deliverable will be checked and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::Cycle { part: b },
            "Part b belongs to a dependency cycle. Remove the cycle and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::FirstPartDepends { part: a },
            "Part a must depend on nothing. Reorder the parts or remove its dependencies and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::UnknownDependency { part: b, dependency: a },
            "Part b depends on unknown part a. Use only parts in this proposal and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::DuplicateDependency { part: b, dependency: a },
            "Part b repeats dependency a. List it once and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::EmptyTitle { part: a },
            "Part a has no title. Name its deliverable and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::EmptyDeliverable { part: a },
            "Part a has no deliverable. State its independent result and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::PartCount { count: 1 },
            "The proposal has 1 parts. Supply between 2 and 26 independently checkable parts and submit the whole proposal again.\n",
        ),
        (
            ProposalFault::AboveLimit {
                part: a,
                estimate: Estimate::new(2000.into(), "pages".to_owned()).unwrap(),
                limit: 1500,
            },
            "Part a estimates 2000 pages, above the limit of 1500. Split it further and submit the whole proposal again.\n",
        ),
    ];
    let actual: Vec<_> = cases
        .iter()
        .map(|(fault, _)| fault_text(&texts, fault).unwrap())
        .collect();
    let expected: Vec<_> = cases.iter().map(|(_, text)| text.to_string()).collect();
    assert_eq!(actual, expected);
}
