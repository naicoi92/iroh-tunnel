//! Run an in-memory quick access configuration.

use std::future::Future;

use anyhow::Result;

use super::skeleton::run_quick;
use super::strategy::AccessStrategy;
use crate::config::AccessConfig;
use crate::role::Role;
use iroh_tunnel_engine::{OnReady, RunContext};

/// Access counterpart of [`super::run_quick_serve`].
///
/// # Errors
///
/// As [`super::run_quick_serve`], for the access role.
pub async fn run_quick_access(
    cfg: AccessConfig,
    ctx: RunContext,
    on_ready: OnReady,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    run_quick::<AccessStrategy>(cfg, Role::Access, "access", ctx, on_ready, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_invalid_quick_config_is_a_config_error() {
        let mut cfg = AccessConfig::default();
        cfg.services.push(crate::config::AccessService {
            name: "BAD NAME".into(),
            node_id: "x".into(),
            protocol: crate::config::Protocol::Tcp,
            host: "127.0.0.1".into(),
            port: 1,
            relay_urls: Vec::new(),
            udp: Default::default(),
        });
        let err = run_quick_access(
            cfg,
            RunContext::new(),
            Box::new(|_| Ok(())),
            std::future::ready(()),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("access quick mode"), "{err:#}");
    }
}
