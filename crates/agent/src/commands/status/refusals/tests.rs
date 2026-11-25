use super::*;
use crate::fixtures::at;

fn agent(name: &str) -> AgentName {
    name.parse().expect("a valid agent name")
}

#[test]
fn no_line_when_no_agent_is_refused() {
    assert_eq!(refused_line(&[], Zone::Utc), None);
}

#[test]
fn the_line_names_each_refused_agent_with_its_return_time_in_name_order() {
    let refused = [(agent("gemini-1"), at(400)), (agent("inv-x"), at(900))];

    let line = refused_line(&refused, Zone::Utc).expect("two agents are refused");

    assert!(line.contains("gemini-1 until "), "{line}");
    assert!(line.contains("inv-x until "), "{line}");
    assert!(
        line.find("gemini-1").expect("named") < line.find("inv-x").expect("named"),
        "{line}"
    );
    assert!(line.ends_with('\n'), "{line:?}");
}
