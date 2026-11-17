//! Regression coverage for a listener retained across a concurrent process spawn.

use std::os::unix::net::UnixListener;

use crate::client::{ClientError, Connection};

#[tokio::test]
async fn a_stale_socket_refuses_connections_while_a_duplicate_listener_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    // A fork retains the same open file description until exec closes its CLOEXEC descriptor.
    let retained = listener.try_clone().unwrap();

    super::close(listener).unwrap();
    let error = Connection::connect(&socket).await.unwrap_err();

    assert!(matches!(error, ClientError::NotRunning { .. }), "{error:?}");
    drop(retained);
}
