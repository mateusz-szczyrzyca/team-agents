//! Which worker of the pool serves a request: one of `[tiers] workers`, of a provider whose turn it is next, that meets
//! the request's `requires`.
//!
//! The workers of `[tiers] workers` are one pool of the installation: a request of any team, or of a solo task whose role
//! may delegate, is served by a free worker of the pool that meets its `requires`, with the providers taking turns across
//! all requests. This module is the pure decision: it reads the workers' names, providers and declared capabilities, the
//! busy facts of each worker (at work now, its runs against the `[limits] runs_per_worker` cap, and an unsettled finished
//! run), the request's required capabilities and the provider of the last request the pool served, and returns the worker
//! to serve and its provider — never starting it.
//!
//! A worker's provider is what hosts its model: for a model that carries one — `cli = "opencode"` and a model with a `/`
//! — the part of the model before the first `/` (so `openrouter/@preset/qwen` is provider `openrouter`); a model without a
//! `/` keeps the `cli` (`opencode`); for any other worker the `cli` is the provider (`claude`, `codex`, `agy`).
//!
//! The providers take turns in the order they first appear in `[tiers] workers`: the decision starts at the provider
//! after the one that served the last request (remembered by the daemon, as [`LastServed`]), wrapping around, and picks
//! the first free worker of the first provider in that rotation that has a free worker meeting the request. A provider
//! with nothing free is skipped, so a request never waits for one.

use crate::config::ModelName;
use crate::decide::slot::RunLimit;
use crate::decide::worker_result::Elsewhere;
use crate::ids::AgentName;
use crate::kinds::CliKind;

/// One worker of the pool as the decision sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolWorker<'a> {
    /// Its name.
    pub name: &'a AgentName,
    /// Its provider, as [`provider_of`] would compute it.
    pub provider: &'a str,
    /// The capabilities it declares; a worker that declares none meets only a request that requires none.
    pub capabilities: &'a [String],
    /// Whether it is at work now and cannot take a request.
    pub busy: bool,
    /// How many runs the worker has going installation-wide, over all teams and solo tasks.
    pub runs: u32,
    /// Whether the worker has a finished run whose result is not settled yet; such a worker cannot take a request.
    pub unsettled: bool,
}

/// What a request asks of a worker: the capabilities it must have, in the request's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requires<'a> {
    /// The capabilities the worker must declare; none means any worker may serve.
    pub capabilities: &'a [String],
}

/// The turn the pool is on: the provider of the last request it actually served, or `None` before any and after a
/// restart of the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastServed<'a>(pub Option<&'a str>);

/// The worker of the pool that serves the next request and its provider: of the first provider after [`LastServed`] in
/// the pool's provider order that has a free worker meeting `requires`, that worker in `workers` order; `None` when no
/// free worker of the pool meets the request. `run_limit` applies installation-wide to every worker.
/// The provider served is returned so the caller can keep [`LastServed`] on.
#[must_use]
pub fn next_pool_worker<'a>(
    workers: &[PoolWorker<'a>],
    requires: &Requires<'_>,
    last_served: LastServed<'_>,
    run_limit: RunLimit,
) -> Option<(&'a AgentName, &'a str)> {
    let providers = provider_order(workers);
    if providers.is_empty() {
        return None;
    }
    let start = match last_served.0 {
        Some(provider) => providers
            .iter()
            .position(|existing| *existing == provider)
            .map_or(0, |index| index + 1),
        None => 0,
    };
    for &provider in providers
        .iter()
        .chain(providers.iter())
        .skip(start)
        .take(providers.len())
    {
        if let Some(worker) = workers
            .iter()
            .find(|worker| available(worker, run_limit) && worker.provider == provider && meets(worker, requires))
        {
            return Some((worker.name, worker.provider));
        }
    }
    None
}

/// The worker of the pool that serves a second try for a request whose first try failed, preferring another provider.
/// Returns `Some((worker_name, provider))` or `None` if no available worker meets `requires`.
#[must_use]
pub fn second_try_pool_worker<'a>(
    workers: &[PoolWorker<'a>],
    requires: &Requires<'_>,
    failed_worker: &AgentName,
    failed_provider: &str,
    run_limit: RunLimit,
) -> Option<(&'a AgentName, &'a str)> {
    for worker in workers {
        if worker.name != failed_worker
            && available(worker, run_limit)
            && worker.provider != failed_provider
            && meets(worker, requires)
        {
            return Some((worker.name, worker.provider));
        }
    }
    for worker in workers {
        if worker.name != failed_worker && available(worker, run_limit) && meets(worker, requires) {
            return Some((worker.name, worker.provider));
        }
    }
    workers
        .iter()
        .find(|worker| worker.name == failed_worker && available(worker, run_limit) && meets(worker, requires))
        .map(|worker| (worker.name, worker.provider))
}

/// Evaluates elsewhere availability for a pool worker request based on pool workers, requirements, and failed provider.
#[must_use]
pub fn second_try_pool_elsewhere(
    workers: &[PoolWorker<'_>],
    requires: &Requires<'_>,
    failed_worker: &AgentName,
    failed_provider: &str,
    run_limit: RunLimit,
) -> Elsewhere {
    match second_try_pool_worker(workers, requires, failed_worker, failed_provider, run_limit) {
        Some((_, provider)) if provider != failed_provider => Elsewhere::Free,
        Some(_) => Elsewhere::SameProvider,
        None => Elsewhere::None,
    }
}

/// Whether a worker can take a request: not at work, no unsettled finished run, and under its `[limits] runs_per_worker`
/// cap, shared by all workers of the pool.
#[must_use]
fn available(worker: &PoolWorker<'_>, run_limit: RunLimit) -> bool {
    let below_limit = match run_limit {
        RunLimit::Unlimited => true,
        RunLimit::AtMost(limit) => worker.runs < limit.get(),
    };
    !worker.busy && !worker.unsettled && below_limit
}

/// The provider of a worker: for `cli = "opencode"` with a model that carries one, the part of the model before the first
/// `/`; for any other worker or a bare model without a `/`, the `cli` itself.
#[must_use]
pub fn provider_of(cli: CliKind, model: Option<&ModelName>) -> &str {
    if cli == CliKind::Opencode
        && let Some(model) = model
        && let Some((provider, _)) = model.as_str().split_once('/')
    {
        return provider;
    }
    cli.as_str()
}

/// Whether `worker` may serve a request with `requires`: every capability of the request is among the worker's declared
/// ones. A request that requires nothing is met by any worker; a worker that declares none meets only such a request.
#[must_use]
pub fn meets(worker: &PoolWorker<'_>, requires: &Requires<'_>) -> bool {
    requires
        .capabilities
        .iter()
        .all(|needed| worker.capabilities.iter().any(|have| have == needed))
}

/// The pool's providers, in the order they first appear in `workers`.
fn provider_order<'a>(workers: &'a [PoolWorker<'a>]) -> Vec<&'a str> {
    let mut order: Vec<&'a str> = Vec::new();
    for worker in workers {
        if !order.contains(&worker.provider) {
            order.push(worker.provider);
        }
    }
    order
}

#[cfg(test)]
mod tests;
