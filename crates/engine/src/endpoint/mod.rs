//! Iroh `Endpoint` construction from engine settings.
//!
//! Implements T-05. Two roles:
//! - **serve**: persistent `SecretKey` resolved by the host.
//! - **access**: ephemeral key, never persisted.
//!
//! Multi-relay: if `relay_urls` is non-empty, the first URL is the home relay
//! (advertised to peers) and the rest are failover candidates. An empty list
//! falls back to the n0 default relay map via `RelayMode::Default`.
//!
//! Based on Page 05 v3 §6 (relay & discovery). Note: iroh 1.0 changed the
//! endpoint builder API vs. earlier drafts — see the API notes below.

mod build_dial_addr;
mod create;
mod home_relay;
mod n0_default_relay_urls;
mod node_id_string;
mod resolve_relay_urls;

pub use build_dial_addr::build_dial_addr;
pub(crate) use create::create_endpoint_with_key;
pub use home_relay::home_relay;
pub use n0_default_relay_urls::n0_default_relay_urls;
pub use node_id_string::node_id_string;
pub use resolve_relay_urls::resolve_relay_urls;
