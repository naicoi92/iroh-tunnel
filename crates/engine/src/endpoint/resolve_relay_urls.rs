use anyhow::{Context, Result};
use iroh::RelayUrl;

use super::n0_default_relay_urls::n0_default_relay_urls;

/// Resolve the relay URLs to dial peers through, from the config form.
///
/// This is the access-side counterpart to `relay_mode_from_urls`: where the
/// latter decides how *this* endpoint uses relays for itself, this one
/// decides which relay URLs to attach to a *dialed peer's* `EndpointAddr`.
///
/// - Empty input → fall back to [`n0_default_relay_urls`] (mirrors
///   `RelayMode::Default`). The access role cannot dial a peer with an empty
///   relay list under the `Minimal` preset — see IROHTUN-44.
/// - Non-empty input → parse each entry as a [`RelayUrl`], surfacing the
///   invalid entry in the error.
///
/// Lifted out of `access::run` so the parsing logic has a single home
/// (previously it was duplicated with `relay_mode_from_urls` and inlined in
/// the access handler).
pub fn resolve_relay_urls(config_urls: &[String]) -> Result<Vec<RelayUrl>> {
    if config_urls.is_empty() {
        return Ok(n0_default_relay_urls());
    }
    config_urls
        .iter()
        .map(|s| {
            s.parse::<RelayUrl>()
                .with_context(|| format!("invalid relay_url: {s}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- resolve_relay_urls + build_dial_addr tests ----

    #[test]
    fn resolve_relay_urls_falls_back_to_n0_defaults_when_empty() {
        // Empty config must return the n0-operated public relays (IROHTUN-44).
        let urls = resolve_relay_urls(&[]).unwrap();
        assert!(!urls.is_empty(), "expected fallback to n0 defaults");
        let hostnames: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
        assert!(
            hostnames.iter().any(|h| h.contains("relay.n0.iroh.link")),
            "no n0 relay hostname in fallback: {hostnames:?}"
        );
    }

    #[test]
    fn resolve_relay_urls_parses_configured_urls() {
        let cfg = vec![
            "https://use1-1.relay.n0.iroh.link.".to_string(),
            "https://euw-1.relay.n0.iroh.link.".to_string(),
        ];
        let urls = resolve_relay_urls(&cfg).unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].to_string(), "https://use1-1.relay.n0.iroh.link./");
    }

    #[test]
    fn resolve_relay_urls_errors_on_invalid_url() {
        let cfg = vec!["not a url".to_string()];
        let err = resolve_relay_urls(&cfg).unwrap_err();
        assert!(format!("{err:#}").contains("invalid relay_url"));
    }
}
