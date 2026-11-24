//! Which runs are given Serena, by the MCP servers they are started with.

use std::collections::BTreeMap;

use agent_core::cli::McpServerKind;

use super::*;

fn servers(kind: McpServerKind) -> BTreeMap<McpServerKind, CommandLine> {
    BTreeMap::from([(kind, CommandLine::try_from(vec!["/bin/server".to_owned()]).unwrap())])
}

#[test]
fn a_run_has_serena_exactly_when_its_own_servers_do() {
    let cases = [
        (servers(McpServerKind::Agentbus), Serena::Without),
        (servers(McpServerKind::Serena), Serena::Given),
        (BTreeMap::new(), Serena::Without),
    ];

    for (given, expected) in cases {
        assert_eq!(Serena::of(&given), expected, "{given:?}");
    }

    let both = BTreeMap::from([
        servers(McpServerKind::Agentbus).into_iter().next().unwrap(),
        servers(McpServerKind::Serena).into_iter().next().unwrap(),
    ]);
    assert_eq!(Serena::of(&both), Serena::Given);
}
