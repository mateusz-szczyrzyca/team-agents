use super::*;
use crate::ids::AgentName;

/// A pool of workers built from `(name, provider, busy)` rows, the capabilities all empty and every other busy fact at
/// its free default (`runs` 0, `runs_limit` 0 = no cap, `unsettled` false).
fn pool(rows: &[(&str, &str, bool)]) -> Vec<PoolWorker<'static>> {
    rows.iter()
        .map(|(name, provider, busy)| PoolWorker {
            name: Box::leak(name.to_string().parse::<AgentName>().unwrap().into()),
            provider: Box::leak(provider.to_string().into_boxed_str()),
            capabilities: &[],
            busy: *busy,
            runs: 0,
            runs_limit: 0,
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
            runs_limit: 0,
            unsettled: false,
        })
        .collect()
}

/// A pool of workers built from `(name, provider, busy, runs, runs_limit, unsettled)` rows, the capabilities all empty.
fn pool_full(rows: &[(&str, &str, bool, u32, u32, bool)]) -> Vec<PoolWorker<'static>> {
    rows.iter()
        .map(|(name, provider, busy, runs, runs_limit, unsettled)| PoolWorker {
            name: Box::leak(name.to_string().parse::<AgentName>().unwrap().into()),
            provider: Box::leak(provider.to_string().into_boxed_str()),
            capabilities: &[],
            busy: *busy,
            runs: *runs,
            runs_limit: *runs_limit,
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
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
    assert_eq!(provider, "deepinfra");
    // After a deepinfra request: openrouter next.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(Some("deepinfra"))).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
    // After an openrouter request: deepinfra again — its first free worker in the pool order serves.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(Some("openrouter"))).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
    assert_eq!(provider, "deepinfra");
}

// Rule: the rotation wraps around the providers.
#[test]
fn the_rotation_wraps_around_the_providers() {
    let workers = pool(&[("w-a", "deepinfra", false), ("w-b", "openrouter", false)]);
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(Some("openrouter"))).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a provider whose free workers are all busy is skipped for the next.
#[test]
fn a_busy_provider_is_skipped_for_the_next() {
    let workers = pool(&[("w-di", "deepinfra", true), ("w-or", "openrouter", false)]);
    // Deepinfra is next but busy: openrouter serves.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
    // A later turn that starts at a busy provider wraps to the free one.
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(Some("deepinfra"))).unwrap();
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
    let (worker, provider) = next_pool_worker(&workers, &requires(&["node"]), LastServed(Some("openrouter"))).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "deepinfra");
}

// Rule: a request that requires a capability is not served by a worker that does not declare it, even when it is next.
#[test]
fn a_request_that_requires_a_capability_is_not_served_by_a_worker_without_it() {
    let workers = pool_caps(&[("w-plain", "openrouter", false, &[])]);
    assert_eq!(next_pool_worker(&workers, &requires(&["node"]), LastServed(None)), None);
}

// Rule: a request that requires nothing is met by any free worker.
#[test]
fn a_request_that_requires_nothing_is_met_by_any_free_worker() {
    let workers = pool_caps(&[
        ("w-plain", "openrouter", false, &[]),
        ("w-node", "deepinfra", false, &["node"]),
    ]);
    // Turn is openrouter (first in order): its plain worker serves.
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker that declares no capability meets only a request that requires none.
#[test]
fn a_worker_that_declares_no_capability_meets_only_a_request_that_requires_none() {
    let workers = pool_caps(&[("w-plain", "openrouter", false, &[])]);
    assert_eq!(
        next_pool_worker(&workers, &requires(&["node"]), LastServed(None)),
        None,
        "a request requiring `node` is unmet by a worker that declares none"
    );
}

// Rule: when no free worker meets the request, no worker serves.
#[test]
fn when_no_free_worker_meets_the_request_none_serves() {
    let workers = pool(&[("w-di", "deepinfra", true), ("w-or", "openrouter", true)]);
    assert_eq!(next_pool_worker(&workers, &requires(&[]), LastServed(None)), None);
}

// Rule: an empty pool serves nothing.
#[test]
fn an_empty_pool_serves_nothing() {
    let workers: [PoolWorker<'_>; 0] = [];
    assert_eq!(next_pool_worker(&workers, &requires(&[]), LastServed(None)), None);
}

// Rule: `opencode` with a model that carries a provider takes the part before the first `/`.
#[test]
fn opencode_with_a_model_carries_the_provider_before_the_slash() {
    assert_eq!(
        provider_of(CliKind::Opencode, Some("openrouter/@preset/qwen")),
        "openrouter"
    );
    assert_eq!(provider_of(CliKind::Opencode, Some("deepinfra/model-x")), "deepinfra");
}

// Rule: a bare model — no `/` — keeps the `cli` as the provider.
#[test]
fn a_bare_model_keeps_the_cli_as_the_provider() {
    assert_eq!(provider_of(CliKind::Opencode, Some("qwen-worker")), "opencode");
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
        assert_eq!(provider_of(cli, Some("openrouter/@preset/qwen")), cli.as_str());
    }
    assert_eq!(provider_of(CliKind::Claude, None), "claude");
}

// Rule: with a single provider, the rotation wraps back to that same provider after it serves.
#[test]
fn a_single_provider_serves_every_turn() {
    let workers = pool(&[("w-a", "deepinfra", false), ("w-b", "deepinfra", false)]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(provider, "deepinfra");
    assert_eq!(worker, name_of(&workers, 0));
    // After a deepinfra serve, the only provider is still deepinfra.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(Some("deepinfra"))).unwrap();
    assert_eq!(provider, "deepinfra");
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker at its `runs_limit` (the global `[limits] runs_per_worker`) is ineligible and is skipped.
#[test]
fn a_worker_at_its_runs_limit_is_skipped() {
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 1, 1, false),
        ("w-or", "openrouter", false, 0, 1, false),
    ]);
    // Deepinfra is first but at its limit of 1 run: openrouter serves.
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 1));
    assert_eq!(provider, "openrouter");
}

// Rule: a global `runs_per_worker` counts across teams, so a worker that is at its limit anywhere is skipped everywhere.
#[test]
fn a_global_runs_per_worker_counts_across_teams() {
    // The worker's runs are counted installation-wide: one run already going, cap of 1, so it cannot take a second.
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 1, 1, false),
        ("w-or", "openrouter", false, 0, 2, false),
    ]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(provider, "openrouter");
    assert_eq!(worker, name_of(&workers, 1));
}

// Rule: a worker below its `runs_limit` is eligible.
#[test]
fn a_worker_below_its_runs_limit_is_eligible() {
    let workers = pool_full(&[("w-di", "deepinfra", false, 0, 2, false)]);
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: `runs_limit` 0 is no limit, so runs never make the worker ineligible.
#[test]
fn a_zero_runs_limit_never_makes_a_worker_ineligible() {
    let workers = pool_full(&[("w-di", "deepinfra", false, 100, 0, false)]);
    let (worker, _) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
    assert_eq!(worker, name_of(&workers, 0));
}

// Rule: a worker with an unsettled finished run is ineligible and is skipped.
#[test]
fn a_worker_with_an_unsettled_result_is_skipped() {
    let workers = pool_full(&[
        ("w-di", "deepinfra", false, 0, 0, true),
        ("w-or", "openrouter", false, 0, 0, false),
    ]);
    let (worker, provider) = next_pool_worker(&workers, &requires(&[]), LastServed(None)).unwrap();
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
        let (worker, provider) = next_pool_worker(&workers, &requires(&[]), last_served).unwrap();
        assert_eq!(provider, "deepinfra", "team {team} starts at the first provider");
        assert_eq!(worker, name_of(&workers, 0));
    }
    // The two teams share the turn: the pair of requests alternates providers globally through one rotation.
    last_served = LastServed(Some("deepinfra"));
    let (_, provider) = next_pool_worker(&workers, &requires(&[]), last_served).unwrap();
    assert_eq!(provider, "openrouter");
    last_served = LastServed(Some("openrouter"));
    let (_, provider) = next_pool_worker(&workers, &requires(&[]), last_served).unwrap();
    assert_eq!(provider, "deepinfra");
}
