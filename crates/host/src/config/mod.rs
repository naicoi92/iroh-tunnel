//! Config operations: schema, persistence and the typed edit/inspect actions
//! the CLI drives. Each exported operation has its own file; `store`,
//! `mutate` and `fixtures` are private infrastructure.

mod add_service;
mod check_config;
mod display_addr;
mod dto;
mod edit;
mod encode_secret_key;
#[cfg(test)]
mod fixtures;
mod identity_state;
mod keygen;
mod list_services;
mod mutate;
mod random_identity;
mod remove_service;
mod require_existing;
mod resolve_config_path;
mod resolve_secret_key;
mod resolve_write_target;
mod set_service;
mod show_config;
mod store;
mod to_settings;
mod types;

pub use add_service::add_service;
pub use check_config::check_config;
pub(crate) use dto::RoleDoc;
pub use dto::{AccessConfig, AccessService, NodeConfig, ServeConfig, ServeService, UdpSettings};
pub use edit::prepare_edit;
pub use encode_secret_key::encode_secret_key;
pub use identity_state::identity_state;
pub use iroh_tunnel_engine::settings::{Protocol, UdpLimits};
pub use keygen::keygen;
pub use list_services::list_services;
pub use random_identity::random_identity;
pub use remove_service::remove_service;
pub use require_existing::require_existing;
pub use resolve_config_path::resolve_config_path;
pub use resolve_secret_key::resolve_secret_key;
pub use resolve_write_target::resolve_write_target;
pub use set_service::set_service;
pub use show_config::show_config;
pub use to_settings::{access_settings, serve_settings};
pub use types::{
    AddService, Disclosure, EditAttempt, EditOutcome, EditRejection, GeneratedIdentity,
    IdentityState, KeygenOutcome, Mutation, RotationConsent, ServiceList, ServiceRow, SetError,
    SetOutcome, SetService, ShownConfig,
};
pub use types::{FieldChange, ServiceField};
