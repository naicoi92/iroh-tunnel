//! The one place service backends run native commands (`systemctl`,
//! `launchctl`, `schtasks`, init scripts) and report user-facing notes.
//!
//! Backends never spawn a process or print: they ask [`Native`] to run a
//! [`Command`] and push a [`ServiceNote`] through the caller's observer at the
//! point where they used to print. The CLI inherits the child's terminal and
//! renders notes; a library caller captures output and decides what to show.

use std::io;
use std::path::PathBuf;
use std::process::Command;

/// How a native command's output is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Collect stdout and stderr into the [`NativeOutput`]; print nothing.
    Capture,
    /// Let the child write to the caller's terminal; the output stays empty.
    Inherit,
}

/// What a service operation needs from its caller: how native output is
/// delivered and where notes go. Notes reach `notes` synchronously, at the
/// point in the operation where they happen.
pub struct ServiceCall<'a> {
    pub output: OutputMode,
    pub notes: &'a mut dyn FnMut(&ServiceNote),
}

impl<'a> ServiceCall<'a> {
    /// A call that captures native output (for callers without a terminal).
    pub fn capture(notes: &'a mut dyn FnMut(&ServiceNote)) -> Self {
        Self {
            output: OutputMode::Capture,
            notes,
        }
    }
}

/// Where an installed registration lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstalledAt {
    /// A unit, plist or init script on disk.
    Path(PathBuf),
    /// A per-user Task Scheduler task (Windows).
    WindowsTask,
}

/// A user-facing message from a service operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceNote {
    /// BusyBox init has no per-user services: a user-scope request installs
    /// the system-wide script.
    BusyboxSystemWide { script: PathBuf },
    /// `launchctl bootstrap` failed; commands that give richer diagnostics.
    LaunchdBootstrapHint {
        target: String,
        domain: String,
        plist: PathBuf,
    },
    /// The service was installed and started.
    Installed { name: String, at: InstalledAt },
    /// The service was removed.
    Uninstalled { name: String },
    /// `stop` found nothing running.
    AlreadyStopped { name: String },
    /// Windows `status`: the role lock says it runs. `owner` is the
    /// described owner (pid, node id).
    RoleRunning { role: String, owner: String },
    /// Windows `status`: the role lock is free.
    RoleNotRunning { role: String },
    /// Windows `status`: the verbose task listing, verbatim.
    TaskListing { text: String },
}

/// The result of one native command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOutput {
    /// The program that ran.
    pub program: String,
    /// Exit code; `None` when the process ended without one (a signal).
    pub status: Option<i32>,
    /// Captured stdout; empty in [`OutputMode::Inherit`].
    pub stdout: Vec<u8>,
    /// Captured stderr; empty in [`OutputMode::Inherit`].
    pub stderr: Vec<u8>,
}

impl NativeOutput {
    /// Whether the command exited with code 0.
    pub(crate) fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// The exit status as the platform's `ExitStatus` printed it, for error
    /// texts. A signal death has no code and reads "terminated by a signal".
    pub(crate) fn status_text(&self) -> String {
        match self.status {
            #[cfg(windows)]
            Some(code) => format!("exit code: {code}"),
            #[cfg(not(windows))]
            Some(code) => format!("exit status: {code}"),
            None => "terminated by a signal".to_owned(),
        }
    }
}

/// Runs a prepared command in the given mode.
pub(crate) type Runner<'a> = dyn FnMut(&mut Command, OutputMode) -> io::Result<NativeOutput> + 'a;

/// Runs native commands for a backend, in the caller's [`OutputMode`].
pub(crate) struct Native<'a> {
    mode: OutputMode,
    runner: &'a mut Runner<'a>,
}

impl<'a> Native<'a> {
    pub(crate) fn new(mode: OutputMode, runner: &'a mut Runner<'a>) -> Self {
        Self { mode, runner }
    }

    /// Run `command` in the caller's mode: inherited for the CLI, captured
    /// otherwise.
    pub(crate) fn run(&mut self, command: &mut Command) -> io::Result<NativeOutput> {
        (self.runner)(command, self.mode)
    }

    /// Run `command` and always capture, whatever the caller's mode. For
    /// commands whose output the backend must read (launchctl `kill`'s
    /// "No process to signal", `schtasks` listings, `whoami`).
    pub(crate) fn output(&mut self, command: &mut Command) -> io::Result<NativeOutput> {
        (self.runner)(command, OutputMode::Capture)
    }
}

/// The production runner: `.status()` to inherit, `.output()` to capture.
pub(crate) fn system_runner(command: &mut Command, mode: OutputMode) -> io::Result<NativeOutput> {
    let program = command.get_program().to_string_lossy().into_owned();
    match mode {
        OutputMode::Inherit => {
            let status = command.status()?;
            Ok(NativeOutput {
                program,
                status: status.code(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
        OutputMode::Capture => {
            let output = command.output()?;
            Ok(NativeOutput {
                program,
                status: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! A recording runner shared by the backend tests.

    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// One observed step: a native command or a delivered note.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Event {
        Call {
            program: String,
            args: Vec<String>,
            mode: OutputMode,
        },
        Note(ServiceNote),
    }

    pub(crate) type Events = Rc<RefCell<Vec<Event>>>;

    pub(crate) fn call(program: &str, args: &[&str], mode: OutputMode) -> Event {
        Event::Call {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            mode,
        }
    }

    pub(crate) fn output(status: i32, stdout: &str, stderr: &str) -> NativeOutput {
        NativeOutput {
            program: String::new(),
            status: Some(status),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// Records every command into `events` and answers from `responses` in
    /// order (exit 0 with no output once they run out).
    pub(crate) struct FakeRunner {
        events: Events,
        responses: VecDeque<NativeOutput>,
    }

    impl FakeRunner {
        pub(crate) fn new(events: &Events, responses: Vec<NativeOutput>) -> Self {
            Self {
                events: Rc::clone(events),
                responses: responses.into(),
            }
        }

        pub(crate) fn run(
            &mut self,
            command: &mut Command,
            mode: OutputMode,
        ) -> io::Result<NativeOutput> {
            let program = command.get_program().to_string_lossy().into_owned();
            let args = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            self.events.borrow_mut().push(Event::Call {
                program: program.clone(),
                args,
                mode,
            });
            let mut response = self
                .responses
                .pop_front()
                .unwrap_or_else(|| output(0, "", ""));
            response.program = program;
            if mode == OutputMode::Inherit {
                response.stdout.clear();
                response.stderr.clear();
            }
            Ok(response)
        }
    }

    pub(crate) fn note_sink(events: &Events) -> impl FnMut(&ServiceNote) {
        let events = Rc::clone(events);
        move |note| events.borrow_mut().push(Event::Note(note.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    #[test]
    fn inherit_mode_leaves_output_empty_and_output_always_captures() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(0, "a", "b"), output(0, "c", "d")]);
        let mut runner = |c: &mut Command, m| fake.run(c, m);
        let mut native = Native::new(OutputMode::Inherit, &mut runner);
        let run = native.run(&mut Command::new("x")).unwrap();
        assert!(run.stdout.is_empty() && run.stderr.is_empty());
        let captured = native.output(&mut Command::new("y")).unwrap();
        assert_eq!(captured.stdout, b"c");
        assert_eq!(
            *events.borrow(),
            vec![
                call("x", &[], OutputMode::Inherit),
                call("y", &[], OutputMode::Capture)
            ]
        );
    }
}
