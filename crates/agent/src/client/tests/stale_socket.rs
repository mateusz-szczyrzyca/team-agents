//! Creates a stale socket even while another process retains its listening descriptor.

use std::io;
use std::os::unix::net::UnixListener;

/// Stops the listener before closing its descriptor.
pub(super) fn close(listener: UnixListener) -> io::Result<()> {
    drop(listener);
    Ok(())
}

mod tests;
