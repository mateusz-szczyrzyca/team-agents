//! Contract tests for matching, filtering and rendering findings.
use super::*;

fn path(value: &str) -> RepoPath {
    RepoPath(value.to_owned())
}
fn at(day: &str) -> Timestamp {
    format!("2026-10-{day}T00:00:00Z").parse().unwrap()
}
fn own() -> TaskId {
    "task-20261127-112701-oezr".parse().unwrap()
}
fn finding(id: u64, paths: &[&str]) -> Finding {
    Finding {
        id: FindingId(NonZeroU64::new(id).unwrap()),
        added_at: at("01"),
        author: "agent-a".to_owned(),
        task: "task-20261126-112701-abcd".parse().unwrap(),
        kind: FindingKind::Trap,
        paths: paths.iter().map(|p| path(p)).collect(),
        text: "A lesson".to_owned(),
    }
}

#[test]
fn finding_ids_parse_and_order_by_number() {
    let parsed = "F-9".parse::<FindingId>();
    assert_eq!(parsed, Ok(FindingId(NonZeroU64::new(9).unwrap())));
    let nine = parsed.expect("valid id");
    let ten: FindingId = "F-10".parse().expect("valid id");
    assert!(ten > nine);
    assert_eq!(ten.to_string(), "F-10");
    assert_eq!(
        "F-18446744073709551615".parse::<FindingId>().unwrap().to_string(),
        "F-18446744073709551615"
    );
    for raw in ["", "9", "f-9", "F-0", "F--1", "F-+1", "F-1x", "F-18446744073709551616"] {
        assert_eq!(
            raw.parse::<FindingId>(),
            Err(InvalidFindingId { value: raw.to_owned() }),
            "{raw}"
        );
    }
}

#[test]
fn repository_paths_normalize_and_reject_invalid_input() {
    for (raw, expected) in [
        ("./crates/a/", "crates/a"),
        ("././docs//é.md", "docs/é.md"),
        ("docs\\guide", "docs/guide"),
    ] {
        assert_eq!(RepoPath::parse(raw), Ok(path(expected)));
    }
    for raw in ["", ".", "./", "/x/y", "../x/y", "x/../y", "C:\\x"] {
        assert_eq!(
            RepoPath::parse(raw),
            Err(InvalidRepoPath { value: raw.to_owned() }),
            "{raw}"
        );
    }
    assert_eq!(RepoPath::parse(&"é".repeat(200)), Ok(path(&"é".repeat(200))));
}

#[test]
fn score_matches_exact_ancestors_and_deep_siblings() {
    for (p, t, expected) in [
        ("crates/a/b/c.rs", "crates/a/b/c.rs", 3),
        ("crates/a/b", "crates/a/b/c.rs", 2),
        ("crates/a/b/c.rs", "crates/a/b", 2),
        ("crates/a/b/c.rs", "crates/a/b/d.rs", 1),
        ("crates/a/c.rs", "crates/a/d.rs", 0),
        ("crates/a/bc", "crates/a/b", 0),
    ] {
        assert_eq!(score(&finding(1, &[p]), &[path(t)]), expected, "{p} vs {t}");
    }
    let mut global = finding(1, &[]);
    global.kind = FindingKind::Environment;
    assert_eq!(score(&global, &[]), 1);
    assert_eq!(score(&finding(1, &[]), &[path("crates/a")]), 0);
    assert_eq!(
        score(
            &finding(1, &["crates/a/b/c.rs", "docs/x"]),
            &[path("docs/x"), path("crates/a/b/d.rs")]
        ),
        3
    );
}

#[test]
fn item_paths_extracts_only_relative_backtick_paths() {
    for (text, expected) in [
        (
            "`crates/agent/src/x.rs:120`, `docs/log.md`.",
            vec![path("crates/agent/src/x.rs"), path("docs/log.md")],
        ),
        ("`make gate`", vec![]),
        ("`/home/me/x`", vec![]),
        ("`https://example.com/a`", vec![]),
        ("`../x/y`", vec![]),
        ("`-x/y` `httpThing/a`", vec![]),
        (
            "`docs/é.md.,;:)` `./docs/a:12)`",
            vec![path("docs/é.md"), path("docs/a")],
        ),
        ("unclosed `docs/x", vec![]),
    ] {
        assert_eq!(item_paths(text), expected, "{text}");
    }
}

#[test]
fn selection_filters_tasks_shown_age_and_disabled_retention() {
    let mut all = vec![
        finding(1, &["docs/x"]),
        finding(2, &["docs/x"]),
        finding(3, &["docs/x"]),
        finding(4, &["docs/x"]),
        finding(5, &["elsewhere/x"]),
    ];
    all[0].task = own();
    all[2].added_at = "2026-09-30T00:00:00Z".parse().unwrap();
    let shown = BTreeSet::from([all[1].id]);
    let run = [path("docs/x")];
    let selected = select_findings(&all, &run, &shown, &own(), at("31"), 30, 8, 3000);
    assert_eq!(
        selected,
        vec![Selected {
            finding: &all[3],
            score: 3
        }]
    );
    assert_eq!(
        select_findings(&all, &run, &shown, &own(), at("31"), 0, 8, 3000),
        vec![]
    );
}

#[test]
fn selection_orders_score_then_time_then_numeric_id() {
    let mut all = vec![
        finding(4, &["crates/a/b"]),
        finding(9, &["crates/a/b"]),
        finding(2, &["crates/a/b/c"]),
        finding(12, &["crates/a/b"]),
    ];
    all[3].added_at = "2026-09-30T00:00:00Z".parse().unwrap();
    let selected = select_findings(
        &all,
        &[path("crates/a/b/c")],
        &BTreeSet::new(),
        &own(),
        at("02"),
        30,
        8,
        3000,
    );
    assert_eq!(
        selected
            .iter()
            .map(|s| (s.finding.id.to_string(), s.score))
            .collect::<Vec<_>>(),
        vec![
            ("F-2".to_owned(), 3),
            ("F-9".to_owned(), 2),
            ("F-4".to_owned(), 2),
            ("F-12".to_owned(), 2)
        ]
    );
}

#[test]
fn selection_applies_count_and_character_budgets_from_the_end() {
    let mut all: Vec<_> = (1..=5).map(|id| finding(id, &["docs/x"])).collect();
    let run = [path("docs/x")];
    let shown = BTreeSet::new();
    let owner = own();
    assert_eq!(
        select_findings(&all, &run, &shown, &owner, at("02"), 30, 2, 3000),
        vec![
            Selected {
                finding: &all[4],
                score: 3
            },
            Selected {
                finding: &all[3],
                score: 3
            }
        ]
    );
    all.truncate(2);
    for f in &mut all {
        f.text = "é".repeat(600);
    }
    assert_eq!(
        select_findings(&all, &run, &shown, &owner, at("02"), 30, 8, 1000),
        vec![Selected {
            finding: &all[1],
            score: 3
        }]
    );
    assert_eq!(
        select_findings(&all, &run, &shown, &owner, at("02"), 30, 8, 1200).len(),
        2
    );
    assert_eq!(
        select_findings(&all, &run, &shown, &owner, at("02"), 30, 8, 599),
        vec![]
    );
    assert_eq!(
        select_findings(&[], &run, &shown, &owner, at("02"), 30, 8, 3000),
        vec![]
    );
}

#[test]
fn rendering_produces_exact_lines_and_local_dates() {
    let first = finding(1, &["crates/a", "docs/x.md"]);
    let mut second = finding(2, &[]);
    second.kind = FindingKind::Environment;
    let selected = [
        Selected {
            finding: &first,
            score: 3,
        },
        Selected {
            finding: &second,
            score: 1,
        },
    ];
    assert_eq!(
        render_findings(&selected, Zone::Utc),
        "- F-1 [trap] crates/a, docs/x.md — A lesson (agent-a, 2026-10-01)\n- F-2 [environment] the repository — A lesson (agent-a, 2026-10-01)"
    );
    let west = Zone::Fixed(time::UtcOffset::from_hms(-1, 0, 0).unwrap());
    assert_eq!(
        render_findings(&selected[..1], west),
        "- F-1 [trap] crates/a, docs/x.md — A lesson (agent-a, 2026-09-30)"
    );
    assert_eq!(render_findings(&[], Zone::Utc), "");
}

#[test]
fn bus_json_round_trips_validated_values_and_rejects_unknown_kind() {
    let expected = finding(7, &["docs/x"]);
    let json = r#"{"id":"F-7","ts":"2026-10-01T00:00:00Z","author":"agent-a","task":"task-20261126-112701-abcd","kind":"trap","paths":["docs/x"],"text":"A lesson"}"#;
    let parsed = serde_json::from_str::<Finding>(json);
    assert!(matches!(parsed, Ok(ref value) if *value == expected), "{parsed:?}");
    assert_eq!(
        serde_json::from_str::<Finding>(&serde_json::to_string(&expected).unwrap()).unwrap(),
        expected
    );
    for (raw, expected) in [
        ("\"trap\"", FindingKind::Trap),
        ("\"flaky_test\"", FindingKind::FlakyTest),
        ("\"environment\"", FindingKind::Environment),
    ] {
        assert_eq!(serde_json::from_str::<FindingKind>(raw).unwrap(), expected);
    }
    assert!(serde_json::from_str::<FindingKind>("\"unknown\"").is_err_and(|e| e.is_data()));
    assert!(serde_json::from_str::<RepoPath>("\"../x\"").is_err_and(|e| e.is_data()));
    assert!(serde_json::from_str::<FindingId>("\"F-0\"").is_err_and(|e| e.is_data()));
}

#[test]
fn selection_keeps_future_findings_and_stops_at_an_oversized_first_text() {
    let mut all = vec![
        finding(1, &["docs/x"]),
        finding(2, &["docs/x"]),
        finding(3, &["docs/x"]),
    ];
    all[2].added_at = at("03");
    all[2].text = "x".repeat(1001);
    all[1].text = "x".repeat(600);
    let run = [path("docs/x")];
    let shown = BTreeSet::new();
    assert_eq!(
        select_findings(&all, &run, &shown, &own(), at("02"), 30, 8, 3000)
            .iter()
            .map(|s| s.finding.id)
            .collect::<Vec<_>>(),
        vec![all[2].id, all[1].id, all[0].id]
    );
    assert_eq!(
        select_findings(&all, &run, &shown, &own(), at("02"), 30, 8, 1000),
        vec![]
    );
    assert_eq!(
        select_findings(&all, &run, &shown, &own(), at("02"), 30, 0, 3000),
        vec![]
    );
}

#[test]
fn run_paths_accept_whitespace_and_long_paths_and_match_ancestors() {
    for raw in ["docs/user guide.md".to_owned(), format!("docs/{}", "é".repeat(201))] {
        let parsed = RepoPath::parse(&raw);
        assert_eq!(parsed, Ok(RepoPath(raw.clone())));
        assert_eq!(score(&finding(1, &["docs"]), &[parsed.expect("valid run path")]), 2);
    }
}

#[test]
fn item_paths_keeps_tokens_longer_than_the_bus_input_limit() {
    let raw = format!("docs/{}", "a".repeat(201));
    assert_eq!(item_paths(&format!("`{raw}`")), vec![RepoPath(raw)]);
}

#[test]
fn rendering_replaces_line_breaks_so_each_finding_occupies_one_line() {
    for separator in ["\n", "\r\n", "\r", "\u{0085}", "\u{2028}", "\u{2029}"] {
        let mut note = finding(1, &["docs"]);
        note.text = format!("First line{separator}Second line");
        let selected = [Selected {
            finding: &note,
            score: 2,
        }];
        let rendered = render_findings(&selected, Zone::Utc);
        assert_eq!(
            rendered,
            "- F-1 [trap] docs — First line Second line (agent-a, 2026-10-01)"
        );
        assert_eq!(rendered.lines().count(), 1);
    }
}
