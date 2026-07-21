//! Shared scaffolding for the serve and access role \`run\` paths.
//!
//! The two roles share ~70% of their skeleton (load config → resolve key →
//! build endpoint → print status → loop → shutdown footer), but the shared
//! pieces were duplicated in \`src/serve.rs\` and \`src/access.rs\`. This module
//! hosts the helpers that are genuinely identical across roles so each role
//! file only carries its own divergence.
//!
//! Everything here is \`pub(crate)\` — these are internal seams between two
//! modules of this crate, not a public extension point.

use iroh::endpoint::Connection;

use crate::config::Protocol;

/// Lowercase protocol name for display (matches the serde form in \`config\`).
///
/// Previously duplicated verbatim in \`serve.rs\` and \`access.rs\`.
pub(crate) fn protocol_str(p: Protocol) -> &'static str {
    match p {
        Protocol::Tcp => "tcp",
        Protocol::Udp => "udp",
    }
}

/// Spawn a task that logs a disconnect line when the peer's QUIC connection
/// closes.
///
/// The weak handle is registered while `conn` is still alive, so iroh
/// guarantees the close event is delivered even if `conn` drops before this
/// resolves. Previously duplicated (with cosmetic differences) in both roles:
/// serve logged \`peer disconnected\` with a \`service\` field, access logged
/// \`disconnected from serve peer\` with an \`sname\` field. Both call sites
/// now build a single \`message\` string and a \`peer\` field, preserving the
/// information without the structural drift.
pub(crate) fn spawn_disconnect_watcher(conn: &Connection, peer: String, message: String) {
    let weak = conn.weak_handle();
    tokio::spawn(async move {
        let _ = weak.closed().await;
        tracing::info!(%peer, "{message}");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_str_matches_serde_form() {
        // Must match the lowercase serde rename on the Protocol enum so the
        // displayed form round-trips through config.
        assert_eq!(protocol_str(Protocol::Tcp), "tcp");
        assert_eq!(protocol_str(Protocol::Udp), "udp");
    }
}
