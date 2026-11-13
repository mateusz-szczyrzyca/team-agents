//! The fixture's own behaviour: what it does when the home is still locked by something that is going away.

use std::time::Duration;

use agent_runtime::store::StateStore;

use super::Daemon;
use crate::fixtures::solo_state;

#[tokio::test]
async fn a_bare_owner_waits_for_a_home_whose_lock_is_released_a_moment_later() {
    let daemon = Daemon::new(&solo_state(), None);
    let holder = StateStore::open(daemon.home.clone()).await.unwrap();
    // The lock of a daemon that ended is kept for a few milliseconds by a child another test forked: here, a store that
    // is dropped a moment after the owner started to open the home.
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(holder);
    });

    let (_owner, _sinks) = daemon.bare_owner().await;

    release.await.unwrap();
}
