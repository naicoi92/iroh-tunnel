//! Running a backend step with its effects and outputs recorded (private).

use anyhow::Result;

use super::native::{Native, Runner, ServiceCall, ServiceNote};
use super::types::{ServiceFailure, ServiceReport};

/// Run a backend step with a [`Native`] over `runner`, keeping the effects and
/// command outputs it recorded whether it succeeded or not.
pub(crate) fn execute(
    call: &mut ServiceCall,
    runner: &mut Runner<'_>,
    step: impl FnOnce(&mut Native, &mut dyn FnMut(&ServiceNote)) -> Result<()>,
) -> Result<ServiceReport, ServiceFailure> {
    let mut native = Native::new(call.output, runner);
    let result = step(&mut native, &mut *call.notes);
    let (applied, outputs) = native.finish();
    match result {
        Ok(()) => Ok(ServiceReport {
            applied,
            native: outputs,
        }),
        Err(error) => Err(ServiceFailure {
            applied,
            native: outputs,
            error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::native::fake::{output, Events, FakeRunner};
    use super::super::types::AppliedEffect;
    use super::*;

    #[test]
    fn a_failing_step_keeps_the_effects_and_outputs_it_recorded() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, vec![output(0, "ok", "")]);
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut notes = |_: &ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let failure = execute(&mut call, &mut runner, |native, _| {
            native.run(&mut std::process::Command::new("tool"))?;
            native.applied(AppliedEffect::Started);
            anyhow::bail!("step failed")
        })
        .unwrap_err();
        assert_eq!(failure.applied, vec![AppliedEffect::Started]);
        assert_eq!(failure.native.len(), 1);
        assert_eq!(failure.error.to_string(), "step failed");
    }

    #[test]
    fn a_successful_step_reports_what_it_did() {
        let events = Events::default();
        let mut fake = FakeRunner::new(&events, Vec::new());
        let mut runner = |c: &mut std::process::Command, m| fake.run(c, m);
        let mut notes = |_: &ServiceNote| {};
        let mut call = ServiceCall::capture(&mut notes);
        let report = execute(&mut call, &mut runner, |native, _| {
            native.applied(AppliedEffect::Enabled);
            Ok(())
        })
        .unwrap();
        assert_eq!(report.applied, vec![AppliedEffect::Enabled]);
    }
}
