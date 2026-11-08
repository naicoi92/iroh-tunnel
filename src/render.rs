//! Text rendering of typed role events for the CLI.

use std::path::Path;

use iroh_tunnel_host::config::{
    GeneratedIdentity, KeygenOutcome, Mutation, ServiceList, SetOutcome,
};
use iroh_tunnel_host::ReadyReport;
use iroh_tunnel_host::Role;

use iroh_tunnel_host::service::{InstalledAt, ServiceNote};
use iroh_tunnel_host::{OnReady, ReadyServices};

/// The stream a rendered line belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

/// Render a service note as the text and stream the CLI printed before notes
/// existed.
pub(crate) fn render_service_note(note: &ServiceNote) -> (Stream, String) {
    match note {
        ServiceNote::BusyboxSystemWide { .. } => (
            Stream::Stderr,
            "note: this host runs BusyBox init (no systemd); the service is a \
             system-wide /etc/init.d script owned by root.\n"
                .to_owned(),
        ),
        ServiceNote::LaunchdBootstrapHint { domain, plist } => (
            Stream::Stderr,
            format!(
                "Hint: a disabled label is the usual cause - check \
                 `launchctl print-disabled {domain}`; run \
                 `launchctl bootstrap {domain} {}` for launchd's own diagnostics.\n",
                plist.display()
            ),
        ),
        ServiceNote::Installed {
            name,
            at: InstalledAt::Path(path),
        } => (
            Stream::Stdout,
            format!("Installed and started {name} at {}\n", path.display()),
        ),
        ServiceNote::Installed {
            name,
            at: InstalledAt::WindowsTask,
        } => (
            Stream::Stdout,
            format!("Installed and started task {name} (runs in your logged-in session)\n"),
        ),
        ServiceNote::Installed {
            name,
            at: InstalledAt::WindowsService,
        } => (
            Stream::Stdout,
            format!("Installed and started service {name} (runs at boot as LocalSystem)\n"),
        ),
        ServiceNote::Uninstalled { name } => (Stream::Stdout, format!("Uninstalled {name}\n")),
        ServiceNote::AlreadyStopped { name } => {
            (Stream::Stdout, format!("{name} is already stopped.\n"))
        }
        ServiceNote::RoleRunning { role, owner } => {
            (Stream::Stdout, format!("{role} is running ({owner})\n"))
        }
        ServiceNote::RoleNotRunning { role } => {
            (Stream::Stdout, format!("{role} is not running\n"))
        }
        ServiceNote::TaskListing { text } => (Stream::Stdout, text.clone()),
    }
}

/// The CLI's service-note observer: print each note at once to its stream, so
/// it stays ordered with the inherited output of native commands.
pub(crate) fn print_service_note(note: &ServiceNote) {
    use std::io::Write as _;
    let (stream, text) = render_service_note(note);
    match stream {
        Stream::Stdout => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        Stream::Stderr => eprint!("{text}"),
    }
}

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
                    svc.protocol.as_str(),
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
                    svc.protocol.as_str(),
                    svc.listen_addr
                ));
            }
        }
    }
    out
}

/// `config set` report: created, the changed fields, or unchanged.
pub(crate) fn render_set(role: Role, outcome: &SetOutcome) -> String {
    match outcome {
        SetOutcome::Created { service, path } => {
            format!("Created service '{service}' in {}.\n", path.display())
        }
        SetOutcome::Updated {
            service,
            path,
            changes,
        } => {
            let mut out = format!("Updated service '{service}' in {}:\n", path.display());
            for c in changes {
                out.push_str(&format!(
                    "  {}: {} -> {}\n",
                    c.field.label(role),
                    c.old,
                    c.new
                ));
            }
            out
        }
        SetOutcome::Unchanged { service, path } => {
            format!("Service '{service}' unchanged ({}).\n", path.display())
        }
    }
}

/// `config check` confirmation line.
pub(crate) fn render_checked(role: Role, path: &Path) -> String {
    format!(
        "OK: {} is a valid {} config.\n",
        path.display(),
        role.name()
    )
}

/// `config rm` confirmation line.
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

/// `config keygen --print` output.
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

/// Render the outcome of `update` as the stdout text the CLI printed before
/// outcomes were typed. `exe_name` is the executable that was asked to update.
pub(crate) fn render_update(
    outcome: &iroh_tunnel_host::update::UpdateOutcome,
    exe_name: &str,
) -> String {
    use iroh_tunnel_host::update::UpdateOutcome;
    match outcome {
        UpdateOutcome::AlreadyCurrent { version } => {
            format!("{exe_name} is already at {version}\n")
        }
        UpdateOutcome::NewerThanLatest { current, latest } => format!(
            "{exe_name} {current} is newer than the latest release {latest}; nothing to do \
             (use --to {latest} to install it anyway)\n"
        ),
        UpdateOutcome::Updated { path, from, to } => format!(
            "updated {} from {from} to {to}\n\
             restart any running tunnel to use the new version (it was not stopped)\n",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh_tunnel_host::config::Protocol;
    use iroh_tunnel_host::{AccessReady, ReadyServices, ServeReady};

    use iroh_tunnel_host::config::{FieldChange, ServiceField};

    fn change(field: ServiceField, old: &str, new: &str) -> FieldChange {
        FieldChange {
            field,
            old: old.into(),
            new: new.into(),
        }
    }

    #[test]
    fn render_created() {
        let outcome = SetOutcome::Created {
            service: "web".into(),
            path: "/x/s.toml".into(),
        };
        assert_eq!(
            render_set(Role::Serve, &outcome),
            "Created service 'web' in /x/s.toml.\n"
        );
    }

    #[test]
    fn render_updated_lists_changes() {
        let outcome = SetOutcome::Updated {
            service: "web".into(),
            path: "/x/s.toml".into(),
            changes: vec![
                change(ServiceField::Address, "127.0.0.1:3000", "127.0.0.1:3001"),
                change(ServiceField::Protocol, "tcp", "udp"),
            ],
        };
        assert_eq!(
            render_set(Role::Serve, &outcome),
            "Updated service 'web' in /x/s.toml:\n  target: 127.0.0.1:3000 -> 127.0.0.1:3001\n  protocol: tcp -> udp\n"
        );
    }

    #[test]
    fn render_unchanged() {
        let outcome = SetOutcome::Unchanged {
            service: "web".into(),
            path: "/x/s.toml".into(),
        };
        assert_eq!(
            render_set(Role::Serve, &outcome),
            "Service 'web' unchanged (/x/s.toml).\n"
        );
    }

    #[test]
    fn render_checked_names_the_role() {
        assert_eq!(
            render_checked(Role::Serve, Path::new("/x/s.toml")),
            "OK: /x/s.toml is a valid serve config.\n"
        );
    }

    #[test]
    fn access_label_is_listen() {
        let outcome = SetOutcome::Updated {
            service: "web".into(),
            path: "/x/a.toml".into(),
            changes: vec![change(ServiceField::Address, "127.0.0.1:1", "127.0.0.1:2")],
        };
        assert!(
            render_set(Role::Access, &outcome).contains("  listen: 127.0.0.1:1 -> 127.0.0.1:2\n")
        );
    }

    #[test]
    fn render_service_note_matches_legacy_text() {
        use iroh_tunnel_host::service::{InstalledAt, ServiceNote};
        let check = |note: ServiceNote, stream: Stream, text: &str| {
            let (got_stream, got) = render_service_note(&note);
            assert_eq!((got_stream, got.as_str()), (stream, text));
        };
        check(
            ServiceNote::BusyboxSystemWide {
                script: "/etc/init.d/S96iroh-tunnel-serve".into(),
            },
            Stream::Stderr,
            "note: this host runs BusyBox init (no systemd); the service is a \
             system-wide /etc/init.d script owned by root.\n",
        );
        check(
            ServiceNote::LaunchdBootstrapHint {
                domain: "gui/501".into(),
                plist: "/p/x.plist".into(),
            },
            Stream::Stderr,
            "Hint: a disabled label is the usual cause - check `launchctl print-disabled gui/501`; \
             run `launchctl bootstrap gui/501 /p/x.plist` for launchd's own diagnostics.\n",
        );
        check(
            ServiceNote::Installed {
                name: "dev.iroh-tunnel.serve".into(),
                at: InstalledAt::Path("/p/x.plist".into()),
            },
            Stream::Stdout,
            "Installed and started dev.iroh-tunnel.serve at /p/x.plist\n",
        );
        check(
            ServiceNote::Installed {
                name: "iroh-tunnel-serve".into(),
                at: InstalledAt::WindowsTask,
            },
            Stream::Stdout,
            "Installed and started task iroh-tunnel-serve (runs in your logged-in session)\n",
        );
        check(
            ServiceNote::Installed {
                name: "iroh-tunnel-serve".into(),
                at: InstalledAt::WindowsService,
            },
            Stream::Stdout,
            "Installed and started service iroh-tunnel-serve (runs at boot as LocalSystem)\n",
        );
        check(
            ServiceNote::Uninstalled {
                name: "iroh-tunnel-serve.service".into(),
            },
            Stream::Stdout,
            "Uninstalled iroh-tunnel-serve.service\n",
        );
        check(
            ServiceNote::AlreadyStopped {
                name: "dev.iroh-tunnel.serve".into(),
            },
            Stream::Stdout,
            "dev.iroh-tunnel.serve is already stopped.\n",
        );
        check(
            ServiceNote::RoleRunning {
                role: "serve".into(),
                owner: "pid 7, node_id N".into(),
            },
            Stream::Stdout,
            "serve is running (pid 7, node_id N)\n",
        );
        check(
            ServiceNote::RoleNotRunning {
                role: "serve".into(),
            },
            Stream::Stdout,
            "serve is not running\n",
        );
        check(
            ServiceNote::TaskListing {
                text: "a\nb".into(),
            },
            Stream::Stdout,
            "a\nb",
        );
    }

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
