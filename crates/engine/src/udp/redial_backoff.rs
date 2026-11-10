use std::time::{Duration, Instant};

/// Fixed pause before redialing a peer that reported unusable datagrams.
const REDIAL_BACKOFF: Duration = Duration::from_secs(30);

/// Token for one seen connection, owned by the UDP loop.
///
/// Never derived from `stable_id()`: a freed allocation address may be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Incarnation(u64);

/// Result of asking whether a redial may proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackoffCheck {
    /// No window is active; dialing may proceed.
    Clear,
    /// A window is active; `first` is true once per window.
    Waiting {
        /// True on the first check of the window, so the caller warns once.
        first: bool,
        /// Time left until the window ends.
        resumes_in: Duration,
    },
}

/// Redial back-off after a peer closed for unsupported datagrams.
#[derive(Debug)]
pub(crate) struct RedialBackoff {
    window: Duration,
    latest: u64,
    failed: Option<u64>,
    until: Option<Instant>,
    warned: bool,
}

impl Default for RedialBackoff {
    fn default() -> Self {
        Self {
            window: REDIAL_BACKOFF,
            latest: 0,
            failed: None,
            until: None,
            warned: false,
        }
    }
}

impl RedialBackoff {
    /// Builds a back-off with a custom window for tests.
    #[cfg(test)]
    pub(crate) fn with_window(window: Duration) -> Self {
        Self {
            window,
            ..Self::default()
        }
    }

    /// Reports the active window end without consuming any state.
    #[cfg(test)]
    pub(crate) fn waiting_until(&self) -> Option<Instant> {
        self.until
    }

    /// Starts a new incarnation and returns its token.
    pub(crate) fn begin(&mut self) -> Incarnation {
        self.latest += 1;
        Incarnation(self.latest)
    }

    /// Records a failure of `incarnation` and opens a window.
    ///
    /// Ignored when the incarnation is stale or already recorded.
    pub(crate) fn record(&mut self, incarnation: Incarnation, now: Instant) {
        if incarnation.0 != self.latest || self.failed == Some(incarnation.0) {
            return;
        }
        self.failed = Some(incarnation.0);
        self.until = Some(now + self.window);
        self.warned = false;
    }

    /// Checks whether dialing may proceed at `now`.
    pub(crate) fn check(&mut self, now: Instant) -> BackoffCheck {
        let Some(until) = self.until else {
            return BackoffCheck::Clear;
        };
        if now >= until {
            self.until = None;
            self.warned = false;
            return BackoffCheck::Clear;
        }
        let first = !self.warned;
        self.warned = true;
        BackoffCheck::Waiting {
            first,
            resumes_in: until - now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_clear_before_any_failure() {
        let t0 = std::time::Instant::now();
        assert_eq!(RedialBackoff::default().check(t0), BackoffCheck::Clear);
    }

    #[test]
    fn backoff_waits_30_seconds_and_warns_once() {
        let t0 = std::time::Instant::now();
        let mut b = RedialBackoff::default();
        let a = b.begin();
        b.record(a, t0);
        assert_eq!(
            b.check(t0),
            BackoffCheck::Waiting {
                first: true,
                resumes_in: REDIAL_BACKOFF
            }
        );
        assert_eq!(
            b.check(t0 + Duration::from_secs(29)),
            BackoffCheck::Waiting {
                first: false,
                resumes_in: Duration::from_secs(1)
            }
        );
        assert_eq!(b.check(t0 + REDIAL_BACKOFF), BackoffCheck::Clear);
    }

    #[test]
    fn backoff_ignores_a_repeat_of_the_same_failure() {
        let t0 = std::time::Instant::now();
        let mut b = RedialBackoff::default();
        let a = b.begin();
        b.record(a, t0);
        let _ = b.check(t0);
        b.record(a, t0 + Duration::from_secs(10));
        assert_eq!(
            b.check(t0 + Duration::from_secs(10)),
            BackoffCheck::Waiting {
                first: false,
                resumes_in: Duration::from_secs(20)
            }
        );
    }

    #[test]
    fn backoff_new_incarnation_failure_starts_a_new_window() {
        let t0 = std::time::Instant::now();
        let mut b = RedialBackoff::default();
        let a = b.begin();
        b.record(a, t0);
        let _ = b.check(t0 + REDIAL_BACKOFF);
        let c = b.begin();
        b.record(c, t0 + REDIAL_BACKOFF);
        assert_eq!(
            b.check(t0 + REDIAL_BACKOFF),
            BackoffCheck::Waiting {
                first: true,
                resumes_in: REDIAL_BACKOFF
            }
        );
    }

    #[test]
    fn backoff_ignores_a_stale_incarnation() {
        let t0 = std::time::Instant::now();
        let mut b = RedialBackoff::default();
        let a = b.begin();
        let _c = b.begin();
        b.record(a, t0);
        assert_eq!(b.check(t0), BackoffCheck::Clear);
    }
}
