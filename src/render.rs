//! Text rendering of typed role events for the CLI.

use std::path::Path;

use crate::config_cmd::{GeneratedIdentity, KeygenOutcome, Mutation, ServiceList};
use crate::ready::ReadyReport;
use crate::status::Role;

use crate::ready::{OnReady, ReadyServices};
use crate::role_run::protocol_str;

/// Render the operator-facing startup lines.
pub(crate) fn render_ready(report: &ReadyReport) -> String {
    let mut out = format!("NodeId: {}\n", report.node_id);
    if let Some(relay) = &report.home_relay {
        out.push_str(&format!("Home relay: {relay}\n"));
    }
    match &report.services {
        ReadyServices::Serve(services) => {
            for svc in services {
                out.push_str(&format!(
                    "Serving: {} {}://{}\n",
                    svc.name,
                    protocol_str(svc.protocol),
                    svc.local_addr
                ));
            }
        }
        ReadyServices::Access(services) => {
            for svc in services {
                let relay_note = if svc.relay_override_count == 0 {
                    String::new()
                } else {
                    format!(" [relay override: {}]", svc.relay_override_count)
                };
                out.push_str(&format!(
                    "Exposed: {} {} -> peer {} ({}://{}){relay_note}\n",
                    svc.name,
                    svc.listen_addr,
                    svc.peer_display,
                    protocol_str(svc.protocol),
                    svc.listen_addr
                ));
            }
        }
    }
    out
}

/// `config add` confirmation line.
pub(crate) fn render_added(outcome: &Mutation) -> String {
    format!(
        "Added service '{}' to {}.\n",
        outcome.service,
        outcome.path.display()
    )
}

/// `config remove` confirmation line.
pub(crate) fn render_removed(outcome: &Mutation) -> String {
    format!(
        "Removed service '{}' from {}.\n",
        outcome.service,
        outcome.path.display()
    )
}

/// `config path` output.
pub(crate) fn render_path(path: &Path) -> String {
    format!("{}\n", path.display())
}

/// `config keygen random` output.
pub(crate) fn render_identity(identity: &GeneratedIdentity) -> String {
    format!(
        "Secret key: {}\nNode ID: {}\n",
        identity.secret(),
        identity.node_id
    )
}

/// `config keygen` output for a written key. Empty for `ConsentNeeded`, which
/// the caller turns into an error.
pub(crate) fn render_keygen(outcome: &KeygenOutcome) -> String {
    match outcome {
        KeygenOutcome::Generated { node_id, .. } => format!(
            "Generated new secret_key.\nNew NodeId: {node_id}\n\
             WARNING: configs that reference this node's node_id must be updated to match.\n"
        ),
        KeygenOutcome::ConsentNeeded { .. } => String::new(),
    }
}

/// `config list` output: the role's table, or the "No services configured"
/// line when the list has no rows array.
pub(crate) fn render_service_list(role: Role, list: &ServiceList) -> String {
    let Some(rows) = &list.rows else {
        return format!("No services configured ({}).\n", list.path.display());
    };
    let text = |v: &Option<String>| v.clone().unwrap_or_else(|| "?".to_owned());
    let mut out = String::new();
    if role == Role::Access {
        out.push_str(&format!(
            "{:<24} {:<14} {:<22} {:<8}\n",
            "NAME", "NODE_ID", "PROTO", "PORT"
        ));
        for r in rows {
            out.push_str(&format!(
                "{:<24} {:<14} {:<22} {:<8}\n",
                text(&r.name),
                text(&r.node_id),
                text(&r.protocol),
                r.port.unwrap_or(0),
            ));
        }
    } else {
        out.push_str(&format!(
            "{:<24} {:<22} {:<18} {:<8}\n",
            "NAME", "PROTO", "HOST", "PORT"
        ));
        for r in rows {
            out.push_str(&format!(
                "{:<24} {:<22} {:<18} {:<8}\n",
                text(&r.name),
                text(&r.protocol),
                text(&r.host),
                r.port.unwrap_or(0),
            ));
        }
    }
    out
}

/// The CLI's readiness hook: print the rendered startup lines to stdout.
pub(crate) fn print_ready() -> OnReady {
    Box::new(|report| {
        print!("{}", render_ready(report));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;
    use crate::ready::{AccessReady, ReadyServices, ServeReady};

    #[test]
    fn render_ready_matches_legacy_lines() {
        let serve = ReadyReport {
            node_id: "X".into(),
            home_relay: Some("R".into()),
            services: ReadyServices::Serve(vec![ServeReady {
                name: "web".into(),
                protocol: Protocol::Tcp,
                local_addr: "127.0.0.1:8080".into(),
            }]),
        };
        assert_eq!(
            render_ready(&serve),
            "NodeId: X\nHome relay: R\nServing: web tcp://127.0.0.1:8080\n"
        );

        let access = ReadyReport {
            node_id: "X".into(),
            home_relay: None,
            services: ReadyServices::Access(vec![AccessReady {
                name: "web".into(),
                listen_addr: "127.0.0.1:9000".into(),
                peer_display: "PEER".into(),
                protocol: Protocol::Udp,
                relay_override_count: 2,
            }]),
        };
        assert_eq!(
            render_ready(&access),
            "NodeId: X\nExposed: web 127.0.0.1:9000 -> peer PEER (udp://127.0.0.1:9000) [relay override: 2]\n"
        );

        let plain = ReadyReport {
            services: ReadyServices::Access(vec![AccessReady {
                name: "w".into(),
                listen_addr: "a:1".into(),
                peer_display: "P".into(),
                protocol: Protocol::Tcp,
                relay_override_count: 0,
            }]),
            ..access
        };
        assert_eq!(
            render_ready(&plain),
            "NodeId: X\nExposed: w a:1 -> peer P (tcp://a:1)\n"
        );
    }
}
