//! Run an in-memory quick serve configuration.

use std::future::Future;

use anyhow::Result;

use super::skeleton::run_quick;
use super::strategy::ServeStrategy;
use crate::config::ServeConfig;
use crate::role::Role;
use iroh_tunnel_engine::{OnReady, RunContext};

/// Run an in-memory quick serve configuration without persisting its
/// identity, until `shutdown` resolves. Claims the serve role like
/// [`super::run_foreground`]; never writes a config.
///
/// # Errors
///
/// [`crate::error::CliError::Config`] when `cfg` is invalid, plus the
/// failures of [`super::run_foreground`].
pub async fn run_quick_serve(
    cfg: ServeConfig,
    ctx: RunContext,
    on_ready: OnReady,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_quick::<ServeStrategy>(cfg, Role::Serve, "serve", ctx, on_ready, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_invalid_quick_config_is_a_config_error() {
        let mut cfg = ServeConfig::default();
        cfg.services.push(crate::config::ServeService {
            name: "BAD NAME".into(),
            protocol: crate::config::Protocol::Tcp,
            host: "127.0.0.1".into(),
            port: 1,
            udp: Default::default(),
        });
        let err = run_quick_serve(
            cfg,
            RunContext::new(),
            Box::new(|_| Ok(())),
            std::future::ready(()),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("serve quick mode"), "{err:#}");
    }
}
