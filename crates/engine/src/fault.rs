//! Fatal errors raised to the owner of shared state (#226).

use tokio::sync::mpsc;

/// Raises a fatal error to one owner — a role run or a connection — which
/// stops, cleans up and reports the first raised error.
///
/// Shared state hands this to every unit that touches it: a poisoned lock
/// there leaves the state without a consistent view, so the state's owner
/// ends instead of recovering it. Role-wide state raises to the role (which
/// returns the error as its run result); connection-scoped state raises to
/// its connection only.
#[derive(Clone, Debug)]
pub(crate) struct Fault(mpsc::Sender<anyhow::Error>);

/// The owner's end of the fault channel.
#[derive(Debug)]
pub(crate) struct Faults(mpsc::Receiver<anyhow::Error>);

/// A fault channel for one owner.
pub(crate) fn channel() -> (Fault, Faults) {
    // One slot: the first fault ends the owner; later ones are only logged.
    let (tx, rx) = mpsc::channel(1);
    (Fault(tx), Faults(rx))
}

impl Fault {
    /// Log `error` and end the owner with it unless an earlier fault already
    /// does.
    pub(crate) fn raise(&self, error: anyhow::Error) {
        // Only the fault that ends the owner is an error; later ones (more
        // users of the same poisoned state before the owner stops) and those
        // raised after it finished are noise.
        let message = format!("{error:#}");
        match self.0.try_send(error) {
            Ok(()) => tracing::error!("ending on unrecoverable state: {message}"),
            Err(_) => tracing::debug!("further unrecoverable state: {message}"),
        }
    }

    /// A fault whose owner is gone, for tests of a single unit.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        channel().0
    }
}

impl Faults {
    /// Resolve with the first raised fault; pending while none is raised.
    pub(crate) async fn first(&mut self) -> anyhow::Error {
        match self.0.recv().await {
            Some(error) => error,
            // Every `Fault` is gone, so none can be raised any more.
            None => std::future::pending().await,
        }
    }

    /// The first fault raised so far, without waiting.
    pub(crate) fn take(&mut self) -> Option<anyhow::Error> {
        self.0.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn the_first_raised_fault_wins() {
        let (fault, mut faults) = channel();
        fault.raise(anyhow::anyhow!("first"));
        fault.clone().raise(anyhow::anyhow!("second"));
        assert_eq!(faults.first().await.to_string(), "first");
        assert!(faults.take().is_none(), "later faults are only logged");
    }

    #[tokio::test]
    async fn no_fault_stays_pending_even_after_every_raiser_is_gone() {
        let (fault, mut faults) = channel();
        drop(fault);
        let waited = tokio::time::timeout(Duration::from_millis(20), faults.first()).await;
        assert!(waited.is_err());
        assert!(faults.take().is_none());
    }
}
