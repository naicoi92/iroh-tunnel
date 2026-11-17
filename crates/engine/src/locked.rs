//! Std mutexes whose poisoning is fatal for their owner (#216, #226).

use std::sync::Mutex;

/// A std mutex reachable only through short, synchronous critical sections.
///
/// No guard escapes: callers pass the section as a closure, so a lock can
/// never be held across an `.await` or handed to another unit. A poisoned
/// lock is never recovered, because the panicked holder may have left the
/// value's invariants broken; every later section fails with [`Poisoned`]
/// and the owning connection or role ends with that error (see
/// [`crate::fault::Fault`]).
///
/// Ending is best-effort cleanup: state behind a poisoned lock is dropped,
/// not walked. For a UDP flow table that means its sockets close at once
/// instead of being quarantined, and a poisoned quarantine set aborts its
/// sinks when the role drops it instead of joining them.
#[derive(Debug, Default)]
pub(crate) struct Locked<T>(Mutex<T>);

impl<T> Locked<T> {
    /// Wrap `value`.
    #[cfg(test)]
    pub(crate) fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// Run `section` on the value under the lock.
    ///
    /// # Errors
    ///
    /// [`Poisoned`] when an earlier holder panicked inside its section.
    pub(crate) fn with<R>(&self, section: impl FnOnce(&mut T) -> R) -> Result<R, Poisoned> {
        let mut guard = self.0.lock().map_err(|_| Poisoned)?;
        Ok(section(&mut guard))
    }
}

/// A lock whose earlier holder panicked mid-section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Poisoned;

impl std::fmt::Display for Poisoned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("lock poisoned by a panicked holder")
    }
}

impl std::error::Error for Poisoned {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_section_reads_and_writes_the_value() {
        let locked = Locked::new(1);
        locked.with(|n| *n += 1).unwrap();
        assert_eq!(locked.with(|n| *n), Ok(2));
    }

    #[test]
    fn a_panicked_section_poisons_every_later_section() {
        let locked = Arc::new(Locked::new(0));
        let holder = locked.clone();
        let panicked = std::thread::spawn(move || {
            holder.with(|_| panic!("holder panics mid-section")).ok();
        })
        .join();
        assert!(panicked.is_err(), "the holder thread panicked");
        assert_eq!(locked.with(|n| *n), Err(Poisoned));
        assert_eq!(locked.with(|n| *n = 5), Err(Poisoned), "writes fail too");
    }
}
