//! Run a freshly written executable despite transient `ETXTBSY` (#206).
//!
//! Linux refuses to exec a file while any process holds it open for writing.
//! Tests in one binary run on parallel threads: when one thread forks a child
//! while another still has a just-written executable open, the child inherits
//! that writer descriptor until its own exec, and an exec of the file fails
//! with "Text file busy". The descriptor is inherited by inode, so writing
//! through a temporary name and renaming does not help; the window is short
//! and closes by itself, so a bounded retry does.

use std::io::ErrorKind;
use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

/// Upper bound on how long a sibling's inherited writer may stay open.
const BUSY_DEADLINE: Duration = Duration::from_secs(10);
const BUSY_BACKOFF: Duration = Duration::from_millis(20);

/// [`Command::output`], retried while the executable is busy.
pub fn output(cmd: &mut Command) -> std::io::Result<Output> {
    retry_while_busy(|| cmd.output())
}

/// [`Command::spawn`], retried while the executable is busy.
pub fn spawn(cmd: &mut Command) -> std::io::Result<Child> {
    retry_while_busy(|| cmd.spawn())
}

fn retry_while_busy<T>(mut run: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let deadline = Instant::now() + BUSY_DEADLINE;
    loop {
        match run() {
            Err(err)
                if err.kind() == ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                std::thread::sleep(BUSY_BACKOFF);
            }
            result => return result,
        }
    }
}
