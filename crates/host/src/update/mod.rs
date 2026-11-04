//! Updating the installed executable from a verified release (DIST-02,
//! issue #80 policy), with ownership detection (issue #96).

mod check_owner;
mod current_target;
mod detect;
#[cfg(test)]
mod fixtures;
mod replace;
mod types;
// The operation file is named after the operation, as the layout rule requires.
#[allow(clippy::module_inception)]
mod update;
mod verify;

pub use check_owner::check_owner;
pub use current_target::current_target;
pub use detect::detect;
pub use types::{
    OverwritePolicy, Owner, OwnerDecision, SignatureVerifier, UpdateContext, UpdateOutcome,
    UpdateRequest, Verification, BASE_URL_ENV, DEFAULT_BASE_URL,
};
pub use update::update;
pub use verify::SigstoreVerifier;
