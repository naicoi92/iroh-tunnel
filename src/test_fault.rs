//! Test-only fault seams (`test-utils` builds): make a configured run fail
//! on demand so the service end-to-end tests can drive the failure paths.
//! Without the feature every function here is the identity.

use std::future::Future;

use iroh_tunnel_host::OnReady;

/// Env: the run fails with exit code 1 after this many milliseconds.
#[cfg(feature = "test-utils")]
const ENV_EXIT_AFTER_MS: &str = "IROH_TUNNEL_TEST_EXIT_AFTER_MS";
/// Env: set to `1` to fail with exit code 1 right before `on_ready`.
#[cfg(feature = "test-utils")]
const ENV_FAIL_BEFORE_READY: &str = "IROH_TUNNEL_TEST_FAIL_BEFORE_READY";

/// Faults requested through the environment.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TestFault {
    pub exit_after: Option<std::time::Duration>,
    pub fail_before_ready: bool,
}

#[cfg(any(test, feature = "test-utils"))]
impl TestFault {
    /// Parse the raw variable values; `None` when no fault is requested.
    pub(crate) fn parse(
        exit_after_ms: Option<&str>,
        fail_before_ready: Option<&str>,
    ) -> Option<Self> {
        let exit_after = exit_after_ms
            .and_then(|ms| ms.parse().ok())
            .map(std::time::Duration::from_millis);
        let fail_before_ready = fail_before_ready == Some("1");
        (exit_after.is_some() || fail_before_ready).then_some(Self {
            exit_after,
            fail_before_ready,
        })
    }
}

/// The faults requested by the environment.
#[cfg(feature = "test-utils")]
pub(crate) fn test_fault() -> Option<TestFault> {
    let exit_after = std::env::var(ENV_EXIT_AFTER_MS).ok();
    let fail = std::env::var(ENV_FAIL_BEFORE_READY).ok();
    TestFault::parse(exit_after.as_deref(), fail.as_deref())
}

/// `on_ready` that fails first when `IROH_TUNNEL_TEST_FAIL_BEFORE_READY=1`.
#[cfg(feature = "test-utils")]
pub(crate) fn wrap_ready(on_ready: OnReady) -> OnReady {
    if test_fault().is_some_and(|f| f.fail_before_ready) {
        return Box::new(|_| anyhow::bail!("test fault: failing before ready"));
    }
    on_ready
}

/// Production build: no seam.
#[cfg(not(feature = "test-utils"))]
pub(crate) fn wrap_ready(on_ready: OnReady) -> OnReady {
    on_ready
}

/// Race `run` against `IROH_TUNNEL_TEST_EXIT_AFTER_MS`: when it elapses first
/// the run is dropped and fails (exit code 1).
#[cfg(feature = "test-utils")]
pub(crate) async fn bound_run(run: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    match test_fault().and_then(|f| f.exit_after) {
        Some(after) => tokio::select! {
            result = run => result,
            () = tokio::time::sleep(after) => anyhow::bail!("test fault: exiting after {after:?}"),
        },
        None => run.await,
    }
}

/// Production build: no seam.
#[cfg(not(feature = "test-utils"))]
pub(crate) async fn bound_run(run: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    run.await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_fault_parses_env() {
        assert_eq!(TestFault::parse(None, None), None);
        assert_eq!(TestFault::parse(Some("x"), Some("0")), None);
        assert_eq!(
            TestFault::parse(Some("250"), None),
            Some(TestFault {
                exit_after: Some(Duration::from_millis(250)),
                fail_before_ready: false
            })
        );
        assert_eq!(
            TestFault::parse(None, Some("1")),
            Some(TestFault {
                exit_after: None,
                fail_before_ready: true
            })
        );
    }
}
