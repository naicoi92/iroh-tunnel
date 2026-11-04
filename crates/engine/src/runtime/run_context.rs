use std::sync::Arc;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::shutdown::stopped;

/// Per-invocation record of the first drain deadline.
///
/// `app::execute` creates one per run and shares clones with the role run
/// future and [`forced_exit`], so the CLI boundary measures its watchdog from
/// the same drain the role started, without any process-global state. Clones
/// share one channel; dropping all clones just leaves the deadline unset.
/// Nothing here spawns a task or installs a signal handler.
#[derive(Clone)]
pub struct RunContext(Arc<watch::Sender<Option<Instant>>>);

impl Default for RunContext {
    fn default() -> Self {
        Self::new()
    }
}

impl RunContext {
    /// A context whose drain has not started.
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(None).0))
    }

    /// Resolve with the deadline of the first drain started in this context.
    ///
    /// Pending until [`begin`] publishes; cancel-safe (dropping the future
    /// only drops its subscription).
    pub async fn drain_deadline(&self) -> Instant {
        let mut rx = self.0.subscribe();
        stopped(&mut rx).await
    }

    /// Record `at` unless a drain deadline is already published (first wins).
    pub(crate) fn publish(&self, at: Instant) {
        self.0.send_if_modified(|published| {
            published.is_none() && {
                *published = Some(at);
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown::{begin, channel};
    use std::time::Duration;

    #[tokio::test]
    async fn begin_publishes_to_its_own_context() {
        let (tx, _rx) = channel();
        let ctx = RunContext::new();
        begin(&tx, &ctx);
        let published = ctx.drain_deadline().await;
        assert_eq!(Some(published), *tx.borrow());
    }

    #[tokio::test]
    async fn two_runs_do_not_share_a_deadline() {
        let (tx1, _rx1) = channel();
        let ctx1 = RunContext::new();
        let ctx2 = RunContext::new();
        begin(&tx1, &ctx1);
        let one_ms = Duration::from_millis(1);
        assert!(tokio::time::timeout(one_ms, ctx1.drain_deadline())
            .await
            .is_ok());
        assert!(tokio::time::timeout(one_ms, ctx2.drain_deadline())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn first_published_deadline_wins() {
        let ctx = RunContext::new();
        let first = Instant::now() + Duration::from_secs(1);
        ctx.publish(first);
        ctx.publish(first + Duration::from_secs(5));
        assert_eq!(ctx.drain_deadline().await, first);
    }
}
