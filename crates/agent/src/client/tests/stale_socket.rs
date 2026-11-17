//! Creates a stale socket even while another process retains its listening descriptor.

use std::io;
use std::net::Shutdown;
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};

/// Stops the listener before closing its descriptor.
pub(super) fn close(listener: UnixListener) -> io::Result<()> {
    // Shutdown affects the shared socket, including descriptors inherited between fork and exec.
    UnixStream::from(OwnedFd::from(listener)).shutdown(Shutdown::Both)
}

mod tests;
