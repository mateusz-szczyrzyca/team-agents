//! A closed part keeps its own pull request when the parent advances.

use super::*;
use crate::state::fixtures::{at, drive, pr, round_id, team};
use crate::state::split::{PartDefinition, SplitPlan};
use crate::state::{PhaseKind, RoundEnd};

#[test]
fn historical_part_details_keep_their_merged_pull_request() {
    let mut team = team(1);
    let parent = "1.2".parse().unwrap();
    let id = round_id(1);
    team.open_round(id.clone(), Some(parent), at(1)).unwrap();
    drive(&mut team, "1.2", PhaseKind::Done);
    let mut wire = serde_json::to_value(&team).unwrap();
    let part = |dependencies| PartDefinition {
        title: "A guide".into(),
        delivers: "An independent guide".into(),
        checks: "Read the guide".into(),
        depends_on: dependencies,
    };
    wire["splits"] = serde_json::to_value(vec![
        SplitPlan::new(
            parent,
            vec![part(vec![]), part(vec!["a".to_owned().try_into().unwrap()])],
        )
        .unwrap(),
    ])
    .unwrap();
    wire["rounds"][0]["part"] = serde_json::json!({"status":"working","key":"a"});
    let mut team: Team = serde_json::from_value(wire).unwrap();
    team.close_round(&id, at(70), RoundEnd::Merged).unwrap();
    assert_eq!(
        RoundDetail::new(&team, team.round(&id).unwrap(), vec![]).pr,
        Some(pr(7))
    );
}
