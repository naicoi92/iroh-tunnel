//! Text rendering of typed role events for the CLI.

use std::path::Path;

use iroh_tunnel_host::config::{
    GeneratedIdentity, KeygenOutcome, Mutation, NodeChange, ServiceList, SetNodeOutcome,
    SetOutcome, TokenChange,
};
use iroh_tunnel_host::ReadyReport;
use iroh_tunnel_host::Role;

use iroh_tunnel_host::service::{shell_quote, InstalledAt, ServiceInvocation, ServiceNote};
use iroh_tunnel_host::{OnReady, ReadyServices, Tier};

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
        ServiceNote::OtherTierRunning { role, other } => (
            Stream::Stderr,
            match other {
                Tier::System => format!(
                    "note: {role} is also running as a system service; manage it with sudo or \
                     from an elevated terminal\n"
                ),
                Tier::User => format!("note: {role} is also running in a user session\n"),
            },
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
        ServiceNote::ManagedBinary {
            role, source, copy, ..
        } => (
            Stream::Stdout,
            format!(
                "Registered a root-owned copy of {} at {} for the {role} service; re-run \
                 `sudo iroh-tunnel {role} service install` after upgrading to update it\n",
                source.display(),
                copy.display()
            ),
        ),
    }
}

/// Render a service note for the executable the person invoked: a managed
/// copy registered for a standalone executable names that executable and the
/// pinned config in its rerun guidance.
pub(crate) fn render_service_note_for_invocation(
    note: &ServiceNote,
    invocation: &ServiceInvocation,
) -> (Stream, String) {
    if let (
        ServiceInvocation::Standalone { executable },
        ServiceNote::ManagedBinary {
            role,
            source,
            copy,
            config,
        },
    ) = (invocation, note)
    {
        let rerun = format!(
            "re-run `sudo {} service install --config {}`",
            shell_quote(&executable.to_string_lossy()),
            shell_quote(&config.to_string_lossy())
        );
        return (
            Stream::Stdout,
            format!(
                "Registered a root-owned copy of {} at {} for the {role} service; {rerun} after \
                 upgrading to update it\n",
                source.display(),
                copy.display()
            ),
        );
    }
    render_service_note(note)
}

/// Print service notes synchronously, preserving native output ordering.
///
/// The note callback cannot fail the service operation it reports on, so a
/// write error is logged instead of propagated.
pub(crate) fn print_service_note(note: &ServiceNote, invocation: &ServiceInvocation) {
    let (stream, text) = render_service_note_for_invocation(note, invocation);
    let written = match stream {
        Stream::Stdout => crate::output::stdout(&text),
        Stream::Stderr => crate::output::stderr(&text),
    };
    if let Err(error) = written {
        tracing::warn!("service note not shown: {error:#}");
    }
}

/// Print the other-tier notice on stderr when `role` also runs in the tier
/// other than this process's. Informational: a probe failure prints nothing
/// and a write failure is logged, so the command always continues. The probe
/// only tests a lock without waiting, so it is safe on a runtime thread.
pub(crate) fn print_other_tier_notice(role: Role) {
    let Some(other) = iroh_tunnel_host::other_tier_running(role, Tier::current()) else {
        return;
    };
    let note = ServiceNote::OtherTierRunning {
        role: role.name().to_owned(),
        other,
    };
    let (_, text) = render_service_note(&note);
    if let Err(error) = crate::output::stderr(&text) {
        tracing::warn!("other-tier notice not shown: {error:#}");
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

/// `config node` report: the changed fields, or unchanged. Token values are
/// never carried by the outcome.
pub(crate) fn render_node(outcome: &SetNodeOutcome) -> String {
    match outcome {
        SetNodeOutcome::Unchanged { path } => {
            format!("Node settings unchanged ({}).\n", path.display())
        }
        SetNodeOutcome::Updated { path, changes } => {
            let mut out = format!("Updated node settings in {}:\n", path.display());
            for c in changes {
                match c {
                    NodeChange::RelayUrls { old, new } => out.push_str(&format!(
                        "  relay_urls: {} -> {}\n",
                        node_list(old),
                        node_list(new)
                    )),
                    NodeChange::RelayToken(t) => out.push_str(match t {
                        TokenChange::Set => "  relay_token: (none) -> <redacted>\n",
                        TokenChange::Replaced => {
                            "  relay_token: <redacted> -> <redacted> (changed)\n"
                        }
                        TokenChange::Deleted => "  relay_token: <redacted> -> (none)\n",
                    }),
                }
            }
            out
        }
    }
}

fn node_list(urls: &[String]) -> String {
    if urls.is_empty() {
        "(default)".to_owned()
    } else {
        format!("[{}]", urls.join(", "))
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
/// A ready line that could not be written fails the start.
pub(crate) fn print_ready() -> OnReady {
    Box::new(|report| crate::output::stdout_strict(&render_ready(report)))
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
             (use --to {latest} --force to install it anyway)\n"
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

    fn node_changes(changes: Vec<NodeChange>) -> SetNodeOutcome {
        SetNodeOutcome::Updated {
            path: "/x/s.toml".into(),
            changes,
        }
    }

    fn urls(u: &[&str]) -> Vec<String> {
        u.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn render_node_updated() {
        let outcome = node_changes(vec![
            NodeChange::RelayUrls {
                old: urls(&["https://old.example"]),
                new: urls(&["https://a.example"]),
            },
            NodeChange::RelayToken(TokenChange::Deleted),
        ]);
        assert_eq!(
            render_node(&outcome),
            "Updated node settings in /x/s.toml:\n  relay_urls: [https://old.example] -> [https://a.example]\n  relay_token: <redacted> -> (none)\n"
        );
    }

    #[test]
    fn render_node_token_replaced() {
        let out = render_node(&node_changes(vec![NodeChange::RelayToken(
            TokenChange::Replaced,
        )]));
        assert!(
            out.contains("  relay_token: <redacted> -> <redacted> (changed)\n"),
            "{out}"
        );
    }

    #[test]
    fn render_node_set_from_none() {
        let out = render_node(&node_changes(vec![
            NodeChange::RelayUrls {
                old: urls(&[]),
                new: urls(&["https://a.example"]),
            },
            NodeChange::RelayToken(TokenChange::Set),
        ]));
        assert!(
            out.contains("  relay_urls: (default) -> [https://a.example]\n"),
            "{out}"
        );
        assert!(
            out.contains("  relay_token: (none) -> <redacted>\n"),
            "{out}"
        );
    }

    #[test]
    fn render_node_unchanged() {
        let outcome = SetNodeOutcome::Unchanged {
            path: "/x/s.toml".into(),
        };
        assert_eq!(
            render_node(&outcome),
            "Node settings unchanged (/x/s.toml).\n"
        );
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
    fn standalone_managed_note_names_actual_executable_and_absolute_config() {
        let note = ServiceNote::ManagedBinary {
            role: "serve".into(),
            source: "/source".into(),
            copy: "/copy".into(),
            config: "/home/a/config file.toml".into(),
        };
        let (_, text) = render_service_note_for_invocation(
            &note,
            &ServiceInvocation::Standalone {
                executable: "/opt/role bin/renamed".into(),
            },
        );
        assert_eq!(
            text,
            "Registered a root-owned copy of /source at /copy for the serve service; re-run \
             `sudo '/opt/role bin/renamed' service install --config \
             '/home/a/config file.toml'` after upgrading to update it\n"
        );
        let (_, combined) = render_service_note_for_invocation(&note, &ServiceInvocation::Combined);
        assert_eq!(combined, render_service_note(&note).1);
    }

    #[test]
    fn launchd_bootstrap_hint_names_print_disabled() {
        for domain in ["system", "gui/501"] {
            let (stream, text) = render_service_note(&ServiceNote::LaunchdBootstrapHint {
                domain: domain.into(),
                plist: "/p/x.plist".into(),
            });
            assert_eq!(stream, Stream::Stderr);
            assert!(
                text.contains(&format!("launchctl print-disabled {domain}")),
                "{text}"
            );
            assert!(
                !text.contains("--system") && !text.contains("--user"),
                "{text}"
            );
        }
    }

    #[test]
    fn render_service_note_matches_legacy_text() {
        use iroh_tunnel_host::service::{InstalledAt, ServiceNote};
        let check = |note: ServiceNote, stream: Stream, text: &str| {
            let (got_stream, got) = render_service_note(&note);
            assert_eq!((got_stream, got.as_str()), (stream, text));
        };
        check(
            ServiceNote::OtherTierRunning {
                role: "serve".into(),
                other: iroh_tunnel_host::Tier::System,
            },
            Stream::Stderr,
            "note: serve is also running as a system service; manage it with sudo or from an \
             elevated terminal\n",
        );
        check(
            ServiceNote::OtherTierRunning {
                role: "access".into(),
                other: iroh_tunnel_host::Tier::User,
            },
            Stream::Stderr,
            "note: access is also running in a user session\n",
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
        check(
            ServiceNote::ManagedBinary {
                role: "serve".into(),
                source: "/home/u/.cargo/bin/iroh-tunnel".into(),
                copy: "/var/lib/iroh-tunnel/bin/serve/iroh-tunnel".into(),
                config: "/home/u/.config/iroh-tunnel/serve.toml".into(),
            },
            Stream::Stdout,
            "Registered a root-owned copy of /home/u/.cargo/bin/iroh-tunnel at \
             /var/lib/iroh-tunnel/bin/serve/iroh-tunnel for the serve service; re-run \
             `sudo iroh-tunnel serve service install` after upgrading to update it\n",
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

    #[test]
    fn render_update_newer_than_latest_suggests_force() {
        let outcome = iroh_tunnel_host::update::UpdateOutcome::NewerThanLatest {
            current: "0.1.0".into(),
            latest: "0.0.5".into(),
        };
        assert_eq!(
            render_update(&outcome, "iroh-tunnel"),
            "iroh-tunnel 0.1.0 is newer than the latest release 0.0.5; nothing to do (use --to 0.0.5 --force to install it anyway)\n"
        );
    }
}
