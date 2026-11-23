//! Which worker of the pool serves a request: one of `[tiers] workers`, of a provider whose turn it is next, that meets
//! the request's `requires`.
//!
//! The workers of `[tiers] workers` are one pool of the installation: a request of any team, or of a solo task whose role
//! may delegate, is served by a free worker of the pool that meets its `requires`, with the providers taking turns across
//! all requests. This module is the pure decision: it reads the workers' names, providers and declared capabilities, the
//! ones that are busy, the request's required capabilities and the provider of the last request the pool served, and
//! returns the worker to serve and its provider — never starting it.
//!
//! A worker's provider is what hosts its model: for a model that carries one — `cli = "opencode"` and a model with a `/`
//! — the part of the model before the first `/` (so `openrouter/@preset/qwen` is provider `openrouter`); a model without a
//! `/` keeps the `cli` (`opencode`); for any other worker the `cli` is the provider (`claude`, `codex`, `agy`).
//!
//! The providers take turns in the order they first appear in `[tiers] workers`: the decision starts at the provider
//! after the one that served the last request (remembered by the daemon, as [`last_served`]), wrapping around, and picks
//! the first free worker of the first provider in that rotation that has a free worker meeting the request. A provider
//! with nothing free is skipped, so a request never waits for one.

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
    /// Whether it is at work or otherwise cannot take a request now.
    pub busy: bool,
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
/// free worker of the pool meets the request. The provider served is returned so the caller can keep [`LastServed`] on.
#[must_use]
pub fn next_pool_worker<'a>(
    workers: &[PoolWorker<'a>],
    requires: &Requires<'_>,
    last_served: LastServed<'_>,
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
    for offset in 0..providers.len() {
        let pick = (start + offset) % providers.len();
        let &provider = providers.get(pick)?;
        if let Some(worker) = workers
            .iter()
            .find(|worker| !worker.busy && worker.provider == provider && meets(worker, requires))
        {
            return Some((worker.name, worker.provider));
        }
    }
    None
}

/// The provider of a worker: for `cli = "opencode"` with a model that carries one, the part of the model before the first
/// `/`; for any other worker or a bare model without a `/`, the `cli` itself.
#[must_use]
pub fn provider_of(cli: CliKind, model: Option<&str>) -> &str {
    if cli == CliKind::Opencode
        && let Some(model) = model
        && let Some((provider, _)) = model.split_once('/')
    {
        return provider;
    }
    cli.as_str()
}

/// Whether `worker` may serve a request with `requires`: every capability of the request is among the worker's declared
/// ones. A request that requires nothing is met by any worker; a worker that declares none meets only such a request.
fn meets(worker: &PoolWorker<'_>, requires: &Requires<'_>) -> bool {
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
