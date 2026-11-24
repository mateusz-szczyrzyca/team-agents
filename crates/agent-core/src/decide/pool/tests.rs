use super::*;
use crate::ids::AgentName;

/// A pool of workers built from `(name, provider, busy)` rows, the capabilities all empty and every other busy fact at
/// its free default (`runs` 0, `unsettled` false).
fn pool(rows: &[(&str, &str, bool)]) -> Vec<PoolWorker<'static>> {
    rows.iter()
        .map(|(name, provider, busy)| PoolWorker {
            name: Box::leak(name.to_string().parse::<AgentName>().unwrap().into()),
            provider: Box::leak(provider.to_string().into_boxed_str()),
            capabilities: &[],
            busy: *busy,
            runs: 0,

            unsettled: false,
        })
        .collect()
}

/// A pool of workers built from `(name, provider, busy, capabilities)` rows, the other busy facts at their free default.
fn pool_caps(rows: &[(&str, &str, bool, &[&str])]) -> Vec<PoolWorker<'static>> {
    rows.iter()
        .map(|(name, provider, busy, capabilities)| PoolWorker {
            name: Box::leak(name.to_string().parse::<AgentName>().unwrap().into()),
            provider: Box::leak(provider.to_string().into_boxed_str()),
            capabilities: Box::leak(
                capabilities
                    .iter()
                    .map(|cap| String::from(*cap))
                    .collect::<Vec<String>>()
                    .into_boxed_slice(),
            ),
            busy: *busy,
            runs: 0,

            unsettled: false,
        })
        .collect()
}

/// A pool of workers built from `(name, provider, busy, runs, unsettled)` rows, the capabilities all empty.
fn pool_full(rows: &[(&str, &str, bool, u32, bool)]) -> Vec<PoolWorker<'static>> {
    rows.iter()
        .map(|(name, provider, busy, runs, unsettled)| PoolWorker {
            name: Box::leak(name.to_string().parse::<AgentName>().unwrap().into()),
            provider: Box::leak(provider.to_string().into_boxed_str()),
            capabilities: &[],
            busy: *busy,
            runs: *runs,

            unsettled: *unsettled,
        })
        .collect()
}

fn requires(capabilities: &[&str]) -> Requires<'static> {
    Requires {
        capabilities: Box::leak(
            capabilities
                .iter()
                .map(|cap| String::from(*cap))
                .collect::<Vec<String>>()
                .into_boxed_slice(),
        ),
    }
}

fn name_of<'a>(workers: &'a [PoolWorker<'a>], index: usize) -> &'a AgentName {
    workers[index].name
}

// Rule: a request is served by a free worker of the provider whose turn it is next, returning that worker and its
// provider; when none has been served yet, the first provider in the order starts.
#[test]
fn a_request_is_served_by_a_worker_of_the_provider_whose_turn_it_is_next() {
    let workers = pool(&[
        ("w-di-a", "deepinfra", false),
        ("w-or", "openrouter", false),
        ("w-di-b", "deepinfra", false),
    ]);
    // No provider served yet: deepinfra (the first in the order) with its first free worker.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
    assert_eq!(provider, "deepinfra");
    // After a deepinfra request: openrouter next.
    let (worker, provider) = next_pool_worker(
        &workers,
        &requires(&[]),
        LastServed(Some("deepinfra")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
    // After an openrouter request: deepinfra again — its first free worker in the pool order serves.
    let (worker, provider) = next_pool_worker(
        &workers,
        &requires(&[]),
        LastServed(Some("openrouter")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(worker, name_of(&workers, 0));
    assert_eq!(provider, "deepinfra");
}

// Rule: the rotation wraps around the providers.
#[test]
fn the_rotation_wraps_around_the_providers() {
    let workers = pool(&[("w-a", "deepinfra", false), ("w-b", "openrouter", false)]);
    let (worker, _) = next_pool_worker(
        &workers,
        &requires(&[]),
        LastServed(Some("openrouter")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a provider whose free workers are all busy is skipped for the next.
#[test]
fn a_busy_provider_is_skipped_for_the_next() {
    let workers = pool(&[("w-di", "deepinfra", true), ("w-or", "openrouter", false)]);
    // Deepinfra is next but busy: openrouter serves.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
    // A later turn that starts at a busy provider wraps to the free one.
    let (worker, _) = next_pool_worker(
        &workers,
        &requires(&[]),
        LastServed(Some("deepinfra")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(worker, name_of(&workers, 1));
}

// Rule: a request that requires a capability is served by a worker that declares it, whoever is next in turn.
#[test]
fn a_request_that_requires_a_capability_is_served_by_a_worker_that_declares_it() {
    let workers = pool_caps(&[
        ("w-plain", "openrouter", false, &[]),
        ("w-node", "deepinfra", false, &["node"]),
        ("w-node-2", "openrouter", false, &["node"]),
    ]);
    // Turn is deepinfra (openrouter served last): the only free worker that declares `node` and is of that turn serves.
    let (worker, provider) = next_pool_worker(
        &workers,
        &requires(&["node"]),
        LastServed(Some("openrouter")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "deepinfra");
}

// Rule: a request that requires a capability is not served by a worker that does not declare it, even when it is next.
#[test]
fn a_request_that_requires_a_capability_is_not_served_by_a_worker_without_it() {
    let workers = pool_caps(&[("w-plain", "openrouter", false, &[])]);
    assert_eq!(
        next_pool_worker(&workers, &requires(&["node"]), LastServed(None), RunLimit::Unlimited),
        None
    );
}

// Rule: a request that requires nothing is met by any free worker.
#[test]
fn a_request_that_requires_nothing_is_met_by_any_free_worker() {
    let workers = pool_caps(&[
        ("w-plain", "openrouter", false, &[]),
        ("w-node", "deepinfra", false, &["node"]),
    ]);
    // Turn is openrouter (first in order): its plain worker serves.
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker that declares no capability meets only a request that requires none.
#[test]
fn a_worker_that_declares_no_capability_meets_only_a_request_that_requires_none() {
    let workers = pool_caps(&[("w-plain", "openrouter", false, &[])]);
    assert_eq!(
        next_pool_worker(&workers, &requires(&["node"]), LastServed(None), RunLimit::Unlimited),
        None,
        "a request requiring `node` is unmet by a worker that declares none"
    );
}

// Rule: when no free worker meets the request, no worker serves.
#[test]
fn when_no_free_worker_meets_the_request_none_serves() {
    let workers = pool(&[("w-di", "deepinfra", true), ("w-or", "openrouter", true)]);
    assert_eq!(
        next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited),
        None
    );
}

// Rule: an empty pool serves nothing.
#[test]
fn an_empty_pool_serves_nothing() {
    let workers: [PoolWorker<'_>; 0] = [];
    assert_eq!(
        next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited),
        None
    );
}

// Rule: `opencode` with a model that carries a provider takes the part before the first `/`.
#[test]
fn opencode_with_a_model_carries_the_provider_before_the_slash() {
    assert_eq!(
        provider_of(
            CliKind::Opencode,
            Some(&"openrouter/@preset/qwen".parse::<ModelName>().unwrap())
        ),
        "openrouter"
    );
    assert_eq!(
        provider_of(
            CliKind::Opencode,
            Some(&"deepinfra/model-x".parse::<ModelName>().unwrap())
        ),
        "deepinfra"
    );
}

// Rule: a bare model — no `/` — keeps the `cli` as the provider.
#[test]
fn a_bare_model_keeps_the_cli_as_the_provider() {
    assert_eq!(
        provider_of(CliKind::Opencode, Some(&"qwen-worker".parse::<ModelName>().unwrap())),
        "opencode"
    );
}

// Rule: a missing model keeps the `cli` as the provider.
#[test]
fn a_missing_model_keeps_the_cli_as_the_provider() {
    assert_eq!(provider_of(CliKind::Opencode, None), "opencode");
}

// Rule: a CLI other than `opencode` is the provider whatever its model.
#[test]
fn a_cli_other_than_opencode_is_the_provider() {
    for cli in [CliKind::Claude, CliKind::Codex, CliKind::Gemini] {
        assert_eq!(
            provider_of(cli, Some(&"openrouter/@preset/qwen".parse::<ModelName>().unwrap())),
            cli.as_str()
        );
    }
    assert_eq!(provider_of(CliKind::Claude, None), "claude");
}

// Rule: with a single provider, the rotation wraps back to that same provider after it serves.
#[test]
fn a_single_provider_serves_every_turn() {
    let workers = pool(&[("w-a", "deepinfra", false), ("w-b", "deepinfra", false)]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited).unwrap();
    assert_eq!(provider, "deepinfra");
    assert_eq!(worker, name_of(&workers, 0));
    // After a deepinfra serve, the only provider is still deepinfra.
    let (worker, provider) = next_pool_worker(
        &workers,
        &requires(&[]),
        LastServed(Some("deepinfra")),
        RunLimit::Unlimited,
    )
    .unwrap();
    assert_eq!(provider, "deepinfra");
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker at its `runs_limit` (the global `[limits] runs_per_worker`) is ineligible and is skipped.
#[test]
fn a_worker_at_its_runs_limit_is_skipped() {
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 1, false),
        ("w-or", "openrouter", false, 0, false),
    ]);
    // Deepinfra is first but at its limit of 1 run: openrouter serves.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::of(1)).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
}

// Rule: a global `runs_per_worker` counts across teams, so a worker that is at its limit anywhere is skipped everywhere.
#[test]
fn a_global_runs_per_worker_counts_across_teams() {
    // The worker's runs are counted installation-wide: one run already going, cap of 1, so it cannot take a second.
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 1, false),
        ("w-or", "openrouter", false, 0, false),
    ]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::of(1)).unwrap();
    assert_eq!(provider, "openrouter");
    assert_eq!(worker, name_of(&workers, 1));
}

// Rule: a worker below its `runs_limit` is eligible.
#[test]
fn a_worker_below_its_runs_limit_is_eligible() {
    let workers = pool_full(&[("w-di", "deepinfra", false, 0, false)]);
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::of(2)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: `runs_limit` 0 is no limit, so runs never make the worker ineligible.
#[test]
fn a_zero_runs_limit_never_makes_a_worker_ineligible() {
    let workers = pool_full(&[("w-di", "deepinfra", false, 100, false)]);
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::of(0)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker with an unsettled finished run is ineligible and is skipped.
#[test]
fn a_worker_with_an_unsettled_result_is_skipped() {
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 0, true),
        ("w-or", "openrouter", false, 0, false),
    ]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None), RunLimit::Unlimited).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
}

// Rule: the turn is shared across teams — the same `LastServed` advances both teams' next pick together, because the
// providers take turns once per served request, not per team.
#[test]
fn two_teams_share_one_global_turn() {
    let workers = pool(&[("w-di-a", "deepinfra", false), ("w-or", "openrouter", false)]);
    let mut last_served = LastServed(None);
    // Both teams start the turn at the first provider, deepinfra.
    for team in ["team-one", "team-two"] {
        let (worker, provider) = next_pool_worker(&workers, &requires(&[]), last_served, RunLimit::Unlimited).unwrap();
        assert_eq!(provider, "deepinfra", "team {team} starts at the first provider");
        assert_eq!(worker, name_of(&workers, 0));
    }
    // The two teams share the turn: the pair of requests alternates providers globally through one rotation.
    last_served = LastServed(Some("deepinfra"));
    let (_, provider) = next_pool_worker(&workers, &requires(&[]), last_served, RunLimit::Unlimited).unwrap();
    assert_eq!(provider, "openrouter");
    last_served = LastServed(Some("openrouter"));
    let (_, provider) = next_pool_worker(&workers, &requires(&[]), last_served, RunLimit::Unlimited).unwrap();
    assert_eq!(provider, "deepinfra");
}

// Characterization (the behavior is already implemented; the tables pin it). Rule: a busy first worker of a provider is
// not a busy provider — a free sibling of the same provider serves next, and the free workers of a provider serve in
// their configured (workers) order.
#[test]
fn a_busy_first_worker_does_not_skip_its_provider_and_a_free_sibling_serves() {
    // Each case: (pool rows, last served, expected worker index, expected provider).
    type Case<'a> = (&'a [(&'a str, &'a str, bool)], Option<&'a str>, usize, &'a str);
    let cases: &[Case<'_>] = &[
        // The first worker of the turn provider is busy; its free sibling of the same provider serves.
        (
            &[("w-or-a", "openrouter", true), ("w-or-b", "openrouter", false)],
            None,
            1,
            "openrouter",
        ),
        // The next provider behind it is free too, but the sibling of the turn provider still serves first.
        (
            &[
                ("w-or-a", "openrouter", true),
                ("w-or-b", "openrouter", false),
                ("w-di", "deepinfra", false),
            ],
            None,
            1,
            "openrouter",
        ),
        // Configured order: with several free workers of the provider, the earliest in the pool order serves.
        (
            &[("w-or-a", "openrouter", false), ("w-or-b", "openrouter", false)],
            None,
            0,
            "openrouter",
        ),
        // A provider never serves with its earliest free worker alone: after deepinfra, the sibling still holds the turn.
        (
            &[
                ("w-or-a", "openrouter", true),
                ("w-or-b", "openrouter", false),
                ("w-di", "deepinfra", false),
            ],
            Some("deepinfra"),
            1,
            "openrouter",
        ),
    ];
    for (case, &(rows, last, index, provider)) in cases.iter().enumerate() {
        let workers = pool(rows);
        let (worker, got) = next_pool_worker(&workers, &requires(&[]), LastServed(last), RunLimit::Unlimited)
            .unwrap_or_else(|| panic!("case {case}: expected a pick"));
        assert_eq!(worker, name_of(&workers, index), "case {case}: worker");
        assert_eq!(got, provider, "case {case}: provider");
    }
}

// Characterization. Rule: a provider is skipped for a request when its free workers cannot meet it — only a free worker
// that declares the required capabilities holds the turn for its provider.
#[test]
fn a_free_provider_that_cannot_meet_the_requires_is_skipped_for_one_that_can() {
    // Each case: (pool rows, required capabilities, last served, expected worker index and provider, or none).
    type Case<'a> = (
        &'a [(&'a str, &'a str, bool, &'a [&'a str])],
        &'a [&'a str],
        Option<&'a str>,
        Option<(usize, &'a str)>,
    );
    let cases: &[Case<'_>] = &[
        // Openrouter is free but cannot `node`; the deepinfra worker that declares it serves.
        (
            &[
                ("w-plain", "openrouter", false, &[]),
                ("w-node", "deepinfra", false, &["node"]),
            ],
            &["node"],
            None,
            Some((1, "deepinfra")),
        ),
        // The same when the rotation wraps from deepinfra back to openrouter: incapable openrouter is still skipped.
        (
            &[
                ("w-plain", "openrouter", false, &[]),
                ("w-node", "deepinfra", false, &["node"]),
            ],
            &["node"],
            Some("deepinfra"),
            Some((1, "deepinfra")),
        ),
        // The capable provider holds the turn when it comes first in the rotation.
        (
            &[
                ("w-node", "deepinfra", false, &["node"]),
                ("w-plain", "openrouter", false, &[]),
            ],
            &["node"],
            None,
            Some((0, "deepinfra")),
        ),
        // When no provider can meet the request, nothing serves.
        (&[("w-plain", "openrouter", false, &[])], &["node"], None, None),
    ];
    for (case, &(rows, needed, last, expected)) in cases.iter().enumerate() {
        let workers = pool_caps(rows);
        let requires = requires(needed);
        match expected {
            Some((index, provider)) => {
                let (worker, got) =
                    next_pool_worker(&workers, &requires, LastServed(last), RunLimit::Unlimited).unwrap();
                assert_eq!(worker, name_of(&workers, index), "case {case}: worker");
                assert_eq!(got, provider, "case {case}: provider");
            }
            None => assert_eq!(
                next_pool_worker(&workers, &requires, LastServed(last), RunLimit::Unlimited),
                None,
                "case {case}: no provider can meet"
            ),
        }
    }
}

// Characterization. Rule: a last served provider that names no provider of the pool (unknown — removed or renamed since)
// starts the rotation at the first provider, like no provider was served before.
#[test]
fn an_unknown_last_provider_starts_the_rotation_at_the_first_provider() {
    // Each case: (pool rows, last served, expected worker index, expected provider).
    type Case<'a> = (&'a [(&'a str, &'a str, bool)], Option<&'a str>, usize, &'a str);
    let cases: &[Case<'_>] = &[
        // An unknown last provider: the first provider of the pool serves.
        (
            &[("w-di", "deepinfra", false), ("w-or", "openrouter", false)],
            Some("ghost"),
            0,
            "deepinfra",
        ),
        // An empty name is unknown too.
        (
            &[("w-di", "deepinfra", false), ("w-or", "openrouter", false)],
            Some(""),
            0,
            "deepinfra",
        ),
        // ... even with more providers in the pool.
        (
            &[
                ("w-di", "deepinfra", false),
                ("w-or", "openrouter", false),
                ("w-ge", "gemini", false),
            ],
            Some("alien"),
            0,
            "deepinfra",
        ),
        // Contrast: a known provider of the pool still advances the rotation.
        (
            &[("w-di", "deepinfra", false), ("w-or", "openrouter", false)],
            Some("deepinfra"),
            1,
            "openrouter",
        ),
        // Contrast: after the last provider of the pool the rotation wraps to the first.
        (
            &[("w-di", "deepinfra", false), ("w-or", "openrouter", false)],
            Some("openrouter"),
            0,
            "deepinfra",
        ),
    ];
    for (case, &(rows, last, index, provider)) in cases.iter().enumerate() {
        let workers = pool(rows);
        let (worker, got) = next_pool_worker(&workers, &requires(&[]), LastServed(last), RunLimit::Unlimited)
            .unwrap_or_else(|| panic!("case {case}: expected a pick"));
        assert_eq!(worker, name_of(&workers, index), "case {case}: worker");
        assert_eq!(got, provider, "case {case}: provider");
    }
}

// Characterization. Rule: capabilities are byte-for-byte strings, matched exactly — a non-ASCII capability is one string
// among others, not a prefix, a suffix or a case-twin of it.
#[test]
fn non_ascii_capabilities_are_matched_exactly_and_byte_for_byte() {
    // Each case: (pool rows, required capabilities, expected worker index, or none).
    type Case<'a> = (
        &'a [(&'a str, &'a str, bool, &'a [&'a str])],
        &'a [&'a str],
        Option<usize>,
    );
    let cases: &[Case<'_>] = &[
        // The identical non-ASCII capability serves.
        (&[("w-ja", "openrouter", false, &["日本語"])], &["日本語"], Some(0)),
        // A shorter string is not the capability, though it is its prefix.
        (&[("w-ja", "openrouter", false, &["日本語"])], &["日本"], None),
        // A longer string is not the capability either, though the capability is its prefix.
        (&[("w-ja", "openrouter", false, &["日本語"])], &["日本語ノード"], None),
        // A byte-twin of another spelling of the word is not the capability.
        (&[("w-cafe", "openrouter", false, &["café"])], &["cafe"], None),
        // Of two siblings, only the one that declares the exact bytes serves.
        (
            &[
                ("w-ja", "openrouter", false, &["日本語"]),
                ("w-short", "deepinfra", false, &["日本"]),
            ],
            &["日本語"],
            Some(0),
        ),
    ];
    for (case, &(rows, needed, expected)) in cases.iter().enumerate() {
        let workers = pool_caps(rows);
        let requires = requires(needed);
        match expected {
            Some(index) => {
                let (worker, _) = next_pool_worker(&workers, &requires, LastServed(None), RunLimit::Unlimited).unwrap();
                assert_eq!(worker, name_of(&workers, index), "case {case}: worker");
            }
            None => assert_eq!(
                next_pool_worker(&workers, &requires, LastServed(None), RunLimit::Unlimited),
                None,
                "case {case}: no exact byte match"
            ),
        }
    }
}

// Adjacent workers must give their provider one turn, regardless of how many models it offers.
#[test]
fn adjacent_workers_of_one_provider_do_not_repeat_its_turn() {
    let workers = pool(&[
        ("w-di-a", "deepinfra", false),
        ("w-di-b", "deepinfra", false),
        ("w-or", "openrouter", false),
    ]);
    for (last, expected) in [(None, 0), (Some("deepinfra"), 2), (Some("openrouter"), 0)] {
        assert_eq!(
            next_pool_worker(&workers, &requires(&[]), LastServed(last), RunLimit::Unlimited),
            Some((workers[expected].name, workers[expected].provider)),
            "last provider: {last:?}"
        );
    }
}

// A partial match must not qualify, even when it includes the first required capability.
#[test]
fn a_worker_must_declare_every_required_capability() {
    for (capabilities, expected) in [
        (&["table", "images"][..], Some(0)),
        (&["table"][..], None),
        (&["images"][..], None),
        (&[][..], None),
    ] {
        let workers = pool_caps(&[("w-di", "deepinfra", false, capabilities)]);
        assert_eq!(
            next_pool_worker(
                &workers,
                &requires(&["table", "images"]),
                LastServed(None),
                RunLimit::Unlimited
            ),
            expected.map(|index| (workers[index].name, workers[index].provider)),
            "declared capabilities: {capabilities:?}"
        );
    }
}

// Rule: second try prefers another provider when one is available and meets requirements.
#[test]
fn second_try_pool_outcome_table() {
    let w1_caps: &[&str] = &["build:rust"];
    let w2_caps: &[&str] = &["build:python"];
    let w3_caps: &[&str] = &["build:rust"];

    // Case 1: Incompatible alternate provider ("p2" has python), compatible same provider ("p1" has rust).
    let workers = pool_caps(&[
        ("w1", "p1", false, w1_caps),
        ("w2", "p2", false, w2_caps),
        ("w3", "p1", false, w3_caps),
    ]);
    let req = requires(&["build:rust"]);
    let failed_w = workers[0].name;
    assert_eq!(
        second_try_pool_elsewhere(&workers, &req, failed_w, "p1", RunLimit::Unlimited),
        Elsewhere::SameProvider
    );
    assert_eq!(
        second_try_pool_worker(&workers, &req, failed_w, "p1", RunLimit::Unlimited),
        Some((workers[2].name, workers[2].provider))
    );

    // Case 2: Compatible alternate provider ("p2" has rust).
    let workers_comp = pool_caps(&[
        ("w1", "p1", false, w1_caps),
        ("w2", "p2", false, w1_caps),
        ("w3", "p1", false, w3_caps),
    ]);
    let failed_w_comp = workers_comp[0].name;
    assert_eq!(
        second_try_pool_elsewhere(&workers_comp, &req, failed_w_comp, "p1", RunLimit::Unlimited),
        Elsewhere::Free
    );
    assert_eq!(
        second_try_pool_worker(&workers_comp, &req, failed_w_comp, "p1", RunLimit::Unlimited),
        Some((workers_comp[1].name, workers_comp[1].provider))
    );

    // Case 3: Incompatible alternate provider and incompatible same provider.
    let workers_none = pool_caps(&[("w1", "p1", false, w2_caps), ("w2", "p2", false, w2_caps)]);
    let failed_w_none = workers_none[0].name;
    assert_eq!(
        second_try_pool_elsewhere(&workers_none, &req, failed_w_none, "p1", RunLimit::Unlimited),
        Elsewhere::None
    );
    assert_eq!(
        second_try_pool_worker(&workers_none, &req, failed_w_none, "p1", RunLimit::Unlimited),
        None
    );

    // Case 4: Compatible workers exist but all are busy or at run limit.
    let workers_busy = pool_full(&[("w1", "p1", true, 0, false), ("w2", "p2", false, 1, false)]);
    let failed_w_busy = workers_busy[0].name;
    assert_eq!(
        second_try_pool_elsewhere(
            &workers_busy,
            &requires(&[]),
            failed_w_busy,
            "p1",
            RunLimit::AtMost(std::num::NonZeroU32::new(1).unwrap())
        ),
        Elsewhere::None
    );
}

#[test]
fn second_try_reuses_the_only_compatible_worker_when_free() {
    for (busy, runs, unsettled, expected) in [
        (false, 0, false, true),
        (true, 0, false, false),
        (false, 1, false, false),
        (false, 0, true, false),
    ] {
        let workers = pool_full(&[("only", "p1", busy, runs, unsettled)]);
        let got = second_try_pool_worker(
            &workers,
            &requires(&[]),
            name_of(&workers, 0),
            "p1",
            RunLimit::AtMost(std::num::NonZeroU32::new(1).unwrap()),
        );
        assert_eq!(got, expected.then_some((name_of(&workers, 0), "p1")));
    }
}
