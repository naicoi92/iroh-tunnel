//! `service status`: registration, liveness and owner facts plus the native
//! tool's output.

use anyhow::Result;

use super::backend::Backend;
use super::native::{system_runner, Native, Runner, ServiceCall, ServiceNote};
use super::types::{
    Environment, Fact, Registration, ServiceFailure, ServiceInvocation, ServiceStatusReport,
};
use crate::role::Role;
#[cfg(not(windows))]
use crate::Tier;

/// `service status` in the caller's tier: the registration, liveness and
/// owner facts of that tier, plus the native tool's output (inherited by the
/// terminal in [`OutputMode::Inherit`](super::OutputMode::Inherit), returned
/// otherwise). A role also running in the other tier is reported as a
/// [`ServiceNote::OtherTierRunning`]; its owner is never read.
///
/// # Errors
///
/// A [`ServiceFailure`] when the native tool cannot run (or, on Windows, the
/// role has no registration). A fact that cannot be read is
/// [`Fact::Unavailable`], not an error.
pub fn status(
    role: Role,
    invocation: &ServiceInvocation,
    env: Environment,
    call: &mut ServiceCall<'_>,
) -> Result<ServiceStatusReport, ServiceFailure> {
    let mut runner = system_runner;
    status_via(role, invocation, env, call, &mut runner)
}

/// [`status`] over an injected command runner.
pub(crate) fn status_via(
    role: Role,
    _invocation: &ServiceInvocation,
    env: Environment,
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
) -> Result<ServiceStatusReport, ServiceFailure> {
    let tier = env.tier();
    let liveness = crate::ownership::probe_tier(role, tier);
    let owner = owner_fact(&liveness);
    // Queried once: the absent-service refusal and the registration fact share
    // the answer. An absent service is exit 5.
    #[cfg(windows)]
    let system_registration = if tier == crate::Tier::System {
        Some(
            super::system_verb::system_registration(role)
                .map_err(ServiceFailure::before_effects)?,
        )
    } else {
        None
    };
    super::verb::note_other_tier(role, tier, &mut *call.notes);
    // The Windows backends print the role's liveness as a note. It comes from
    // the probe above, so the note and the report cannot disagree; a failed
    // probe gives no note and stays an unavailable fact in the report.
    #[cfg(windows)]
    let liveness_note = liveness.as_ref().ok().map(|seen| role_note(role, seen));
    #[cfg(not(windows))]
    let liveness_note: Option<ServiceNote> = None;
    let mut native = Native::new(call.output, runner);
    let output = Backend::select(tier).and_then(|backend| {
        backend.status(
            &mut native,
            &mut *call.notes,
            liveness_note.as_ref(),
            role,
            tier,
        )
    });
    let (applied, outputs) = native.finish();
    let output = output.map_err(|error| ServiceFailure {
        applied,
        native: outputs,
        error,
    })?;
    // On Windows the backends refuse an unregistered task or service, so
    // reaching here means it exists.
    #[cfg(windows)]
    let registration = if let Some(registration) = system_registration {
        Fact::Known(registration)
    } else {
        Fact::Known(Registration::Present {
            location: "scheduled task".to_owned(),
        })
    };
    #[cfg(not(windows))]
    let registration = Fact::from_result(registration_of(role.name(), tier));
    Ok(ServiceStatusReport {
        registration,
        liveness: Fact::from_result(liveness),
        owner,
        native: Some(output),
    })
}

/// The owner fact for a liveness probe: a stopped role has a known absent
/// owner; a running role without a readable record, or a failed probe, is
/// unavailable.
fn owner_fact(
    liveness: &anyhow::Result<crate::ownership::Liveness>,
) -> Fact<Option<crate::ownership::OwnerRecord>> {
    match liveness {
        Ok(crate::ownership::Liveness::Running {
            owner: Some(owner), ..
        }) => Fact::Known(Some(owner.clone())),
        Ok(crate::ownership::Liveness::Running { owner: None, .. }) => {
            Fact::Unavailable("the owner record is missing or unreadable".to_owned())
        }
        Ok(crate::ownership::Liveness::NotRunning) => Fact::Known(None),
        Err(error) => Fact::Unavailable(error.to_string()),
    }
}

/// Whether the platform's registration file of `tier` exists (Unix backends).
#[cfg(not(windows))]
fn registration_of(role: &str, tier: Tier) -> Result<Registration> {
    let path = Backend::select(tier)?.registration_path(role, tier)?;
    Ok(if path.exists() {
        Registration::Present {
            location: path.display().to_string(),
        }
    } else {
        Registration::Absent
    })
}

/// The note telling whether `role` runs, from one liveness probe.
#[cfg(any(windows, test))]
fn role_note(role: Role, liveness: &crate::ownership::Liveness) -> ServiceNote {
    match liveness {
        crate::ownership::Liveness::Running { owner, .. } => ServiceNote::RoleRunning {
            role: role.name().to_owned(),
            owner: crate::ownership::describe_owner(owner.as_ref()),
        },
        crate::ownership::Liveness::NotRunning => ServiceNote::RoleNotRunning {
            role: role.name().to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::super::fixtures::{in_isolated_home, run_status};
    use super::*;
    use crate::ownership::{Liveness, OwnerRecord};
    use crate::Tier;

    #[test]
    fn role_note_describes_the_probed_owner() {
        let liveness = Liveness::Running {
            tier: Tier::User,
            owner: Some(OwnerRecord {
                role: "serve".to_owned(),
                pid: 42,
                node_id: None,
                started_at: 0,
            }),
        };
        assert_eq!(
            role_note(Role::Serve, &liveness),
            ServiceNote::RoleRunning {
                role: "serve".to_owned(),
                owner: "pid 42".to_owned(),
            }
        );
    }

    #[test]
    fn role_note_reports_a_free_lock_as_not_running() {
        assert_eq!(
            role_note(Role::Access, &Liveness::NotRunning),
            ServiceNote::RoleNotRunning {
                role: "access".to_owned(),
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_foreground_role_without_registration() {
        const NAME: &str =
            "service::status::tests::status_reports_foreground_role_without_registration";
        in_isolated_home(NAME, |_| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            let report = run_status(vec![]).unwrap();
            assert!(matches!(
                report.registration,
                Fact::Known(Registration::Absent)
            ));
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    fn record() -> OwnerRecord {
        OwnerRecord {
            role: "serve".to_owned(),
            pid: 42,
            node_id: None,
            started_at: 0,
        }
    }

    #[test]
    fn owner_fact_is_known_absent_when_not_running() {
        let fact = owner_fact(&Ok(Liveness::NotRunning));
        assert!(matches!(&fact, Fact::Known(None)), "{fact:?}");
    }

    #[test]
    fn owner_fact_is_the_record_when_running() {
        let fact = owner_fact(&Ok(Liveness::Running {
            tier: Tier::User,
            owner: Some(record()),
        }));
        assert!(
            matches!(&fact, Fact::Known(Some(r)) if r.pid == 42),
            "{fact:?}"
        );
    }

    #[test]
    fn owner_fact_is_unavailable_when_running_without_a_record() {
        let fact = owner_fact(&Ok(Liveness::Running {
            tier: Tier::User,
            owner: None,
        }));
        assert!(matches!(&fact, Fact::Unavailable(_)), "{fact:?}");
    }

    #[test]
    fn owner_fact_is_unavailable_when_the_probe_fails() {
        let fact = owner_fact(&Err(anyhow::anyhow!("lock probe boom")));
        assert!(
            matches!(&fact, Fact::Unavailable(m) if m == "lock probe boom"),
            "{fact:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_not_running_owner_as_known_absent() {
        const NAME: &str =
            "service::status::tests::status_reports_not_running_owner_as_known_absent";
        in_isolated_home(NAME, |_| {
            let report = run_status(vec![]).expect("a stopped role is not a failure");
            assert!(
                matches!(report.liveness, Fact::Known(Liveness::NotRunning)),
                "{report:?}"
            );
            assert!(matches!(report.owner, Fact::Known(None)), "{report:?}");
        });
    }

    /// Claim serve in `tier` and publish `node_id` as its identity.
    #[cfg(all(unix, feature = "test-utils"))]
    fn claim_as(tier: Tier, node_id: &str) -> crate::ownership::RoleClaim {
        let name = if tier == Tier::System {
            "system"
        } else {
            "user"
        };
        // SAFETY: single test thread in an isolated child process.
        unsafe { std::env::set_var("IROH_TUNNEL_TEST_TIER", name) };
        let claim = crate::ownership::claim(Role::Serve).unwrap();
        claim.publish_identity(node_id).unwrap();
        claim
    }

    #[cfg(all(unix, feature = "test-utils"))]
    #[test]
    fn status_reports_own_tier_owner_when_both_run() {
        const NAME: &str = "service::status::tests::status_reports_own_tier_owner_when_both_run";
        in_isolated_home(NAME, |home| {
            #[cfg(target_os = "linux")]
            if crate::service::platform::system_wide_only() {
                return; // BusyBox status runs the real init script
            }
            // SAFETY: single test thread in an isolated child process.
            unsafe { std::env::set_var("IROH_TUNNEL_MACHINE_LOCK_DIR", home.join("machine")) };
            let _system = claim_as(Tier::System, "system-node");
            let _user = claim_as(Tier::User, "user-node");
            // SAFETY: as above.
            unsafe { std::env::remove_var("IROH_TUNNEL_TEST_TIER") };
            let events = crate::service::native::fake::Events::default();
            let mut fake = crate::service::native::fake::FakeRunner::new(&events, vec![]);
            let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
            let mut notes = crate::service::native::fake::note_sink(&events);
            let mut call = ServiceCall::capture(&mut notes);
            let report = status_via(
                Role::Serve,
                &ServiceInvocation::Combined,
                super::super::fixtures::BATCH,
                &mut call,
                &mut runner,
            )
            .unwrap();
            assert!(
                matches!(&report.owner, Fact::Known(Some(o)) if o.node_id.as_deref() == Some("user-node")),
                "{report:?}"
            );
            assert!(
                matches!(
                    report.liveness,
                    Fact::Known(Liveness::Running {
                        tier: Tier::User,
                        ..
                    })
                ),
                "{report:?}"
            );
            assert!(events
                .borrow()
                .contains(&crate::service::native::fake::Event::Note(
                    ServiceNote::OtherTierRunning {
                        role: "serve".into(),
                        other: Tier::System,
                    }
                )));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_missing_owner_as_unavailable() {
        const NAME: &str = "service::status::tests::status_reports_missing_owner_as_unavailable";
        in_isolated_home(NAME, |home| {
            let _claim = crate::ownership::claim(Role::Serve).unwrap();
            std::fs::remove_file(home.join(".local/state/iroh-tunnel/serve.owner.json")).unwrap();
            let report = run_status(vec![]).expect("a missing owner is not a failure");
            assert!(matches!(report.owner, Fact::Unavailable(_)), "{report:?}");
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::Running { .. })
            ));
        });
    }

    #[cfg(unix)]
    #[test]
    fn status_does_not_parse_native_prose() {
        const NAME: &str = "service::status::tests::status_does_not_parse_native_prose";
        in_isolated_home(NAME, |_| {
            // Nothing runs the role, whatever the localized native text says.
            let prose = crate::service::native::fake::output(0, "aktiv (läuft)\n", "");
            let report = run_status(vec![prose]).unwrap();
            assert!(matches!(
                report.liveness,
                Fact::Known(crate::ownership::Liveness::NotRunning)
            ));
            let native = report.native.expect("native output is returned");
            assert_eq!(native.stdout, "aktiv (läuft)\n".as_bytes());
        });
    }
}
