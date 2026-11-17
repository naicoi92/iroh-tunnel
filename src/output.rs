//! CLI output through [`std::io::Write`]: `print!`/`eprint!` panic on a
//! write error, so every stdout/stderr write goes through here (#216).
//!
//! Read-only command output treats a closed stdout pipe as a normal stop
//! (`iroh-tunnel serve config list | head`); every other write error
//! propagates with context.

use std::io::{self, Write};

use anyhow::{Context, Result};

/// Write command output to stdout; a closed pipe is a clean stop.
///
/// # Errors
///
/// Any write failure other than [`io::ErrorKind::BrokenPipe`].
pub(crate) fn stdout(text: &str) -> Result<()> {
    stdout_with_context(text, "failed to write to stdout")
}

/// [`stdout`] with a command-specific error `context`.
///
/// # Errors
///
/// Any write failure other than [`io::ErrorKind::BrokenPipe`].
pub(crate) fn stdout_with_context(text: &str, context: &'static str) -> Result<()> {
    write_tolerating_closed_pipe(&mut io::stdout().lock(), text, context)
}

/// Write text to stdout that must be delivered, e.g. the ready line a
/// supervisor waits for: a closed pipe is an error too.
///
/// # Errors
///
/// Any write failure, including a closed pipe.
pub(crate) fn stdout_strict(text: &str) -> Result<()> {
    write_text(&mut io::stdout().lock(), text).context("failed to write to stdout")
}

/// Write a diagnostic (warning, prompt) to stderr.
///
/// # Errors
///
/// Any write failure.
pub(crate) fn stderr(text: &str) -> Result<()> {
    write_text(&mut io::stderr().lock(), text).context("failed to write to stderr")
}

/// Last-resort diagnostic on stderr where no caller can act on a failure
/// (the final error line before exiting, a notice from a host callback).
/// The exit status still carries the outcome, so a write error is dropped.
pub(crate) fn report(text: &str) {
    let _ = write_text(&mut io::stderr().lock(), text);
}

fn write_text(out: &mut impl Write, text: &str) -> io::Result<()> {
    out.write_all(text.as_bytes())?;
    out.flush()
}

fn write_tolerating_closed_pipe(
    out: &mut impl Write,
    text: &str,
    context: &'static str,
) -> Result<()> {
    match write_text(out, text) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.context(context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer whose every write and flush fails with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn a_closed_pipe_is_a_clean_stop_for_command_output() {
        let result =
            write_tolerating_closed_pipe(&mut Failing(io::ErrorKind::BrokenPipe), "x", "ctx");
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn other_write_errors_propagate_with_context() {
        let err =
            write_tolerating_closed_pipe(&mut Failing(io::ErrorKind::StorageFull), "x", "ctx")
                .unwrap_err();
        assert_eq!(format!("{err}"), "ctx");
        assert_eq!(
            err.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::StorageFull)
        );
    }

    #[test]
    fn strict_writes_report_a_closed_pipe() {
        let err = write_text(&mut Failing(io::ErrorKind::BrokenPipe), "x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn text_is_written_verbatim() {
        let mut buf = Vec::new();
        write_tolerating_closed_pipe(&mut buf, "a\nb\n", "ctx").unwrap();
        assert_eq!(buf, b"a\nb\n");
    }
}
