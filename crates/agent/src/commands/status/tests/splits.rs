//! Golden lines for all four durable part states.

use super::*;

#[test]
fn status_shows_done_active_dependency_waiting_and_ready_parts() {
    let summary = TeamSummary::from(&crate::fixtures::team_state().teams()[0]);
    let mut wire = serde_json::to_value(summary).unwrap();
    let part = |title, depends| {
        serde_json::json!({
            "title":title,"delivers":"An independent guide","checks":"Read the guide","depends_on":depends
        })
    };
    wire["splits"] = serde_json::json!([{"parent":"1.1","parts":[
        part("Résumé", vec![]),part("Second", vec!["a"]),part("Third", vec!["b"]),part("Fourth", vec!["a"])
    ]}]);
    let round = serde_json::json!({
        "id":"round-20261111-153001-q1w2","item":"1.1","opened_at":crate::fixtures::at(1),
        "state":{"status":"closed","at":crate::fixtures::at(2),"how":"merged"},
        "part":{"status":"merged","key":"a","pr":crate::fixtures::pr(1)}
    });
    let mut active = round.clone();
    active["id"] = "round-20261111-153002-q1w2".into();
    active["state"] = serde_json::json!({"status":"open"});
    active["part"] = serde_json::json!({"status":"working","key":"b"});
    wire["rounds"] = serde_json::json!([round, active]);
    let parsed = serde_json::from_value::<TeamSummary>(wire);
    let actual = parsed.as_ref().map_or_else(
        |_| String::new(),
        |team| {
            render(std::slice::from_ref(team))
                .lines()
                .filter(|line| line.starts_with("    1.1"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        },
    );
    assert_eq!(actual, include_str!("splits.txt"));
    parsed.unwrap();
}
