//! Argument vocabulary shared by quick mode and `config set`: address, protocol
//! and color value types plus the per-role service flag groups.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::num::NonZeroU16;

use clap::builder::NonEmptyStringValueParser;
use clap::{Args, ValueEnum};
use iroh_tunnel_host::config::Protocol;

/// A local address: `PORT` (host omitted) or `HOST:PORT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceAddr {
    /// Host as typed; `None` means the default `127.0.0.1`. IPv6 is unbracketed.
    pub host: Option<String>,
    /// Non-zero port.
    pub port: NonZeroU16,
}

fn parse_port(s: &str) -> Result<NonZeroU16, String> {
    s.parse::<NonZeroU16>()
        .map_err(|_| format!("invalid port '{s}': expected a number from 1 to 65535"))
}

/// Parse `PORT`, `HOST:PORT` or `[IPV6]:PORT`.
pub fn parse_service_addr(s: &str) -> Result<ServiceAddr, String> {
    let bad =
        || format!("invalid address '{s}': expected PORT, HOST:PORT or [IPV6]:PORT (port 1-65535)");
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(ServiceAddr {
            host: None,
            port: parse_port(s)?,
        });
    }
    if let Some(rest) = s.strip_prefix('[') {
        let (inside, port) = rest.split_once("]:").ok_or_else(bad)?;
        inside.parse::<Ipv6Addr>().map_err(|_| bad())?;
        return Ok(ServiceAddr {
            host: Some(inside.to_owned()),
            port: parse_port(port)?,
        });
    }
    let (host, port) = s.rsplit_once(':').ok_or_else(bad)?;
    let host_ok = !host.is_empty()
        && !host.contains(':')
        && (host.parse::<Ipv4Addr>().is_ok()
            || host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'));
    if !host_ok {
        return Err(bad());
    }
    Ok(ServiceAddr {
        host: Some(host.to_owned()),
        port: parse_port(port)?,
    })
}

/// Forwarding protocol flag value.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolArg {
    /// TCP streams.
    Tcp,
    /// UDP datagrams.
    Udp,
}

impl From<ProtocolArg> for Protocol {
    fn from(p: ProtocolArg) -> Self {
        match p {
            ProtocolArg::Tcp => Protocol::Tcp,
            ProtocolArg::Udp => Protocol::Udp,
        }
    }
}

/// `--color` value.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorArg {
    /// Color when stderr is a terminal.
    Auto,
    /// Always color.
    Always,
    /// Never color.
    Never,
}

/// Service flags of the Serve role.
#[derive(Args, Debug)]
pub struct ServeServiceArgs {
    /// Service name (lowercase letters, digits, '-'; must match on both sides).
    #[arg(short = 'n', long, value_parser = NonEmptyStringValueParser::new())]
    pub name: Option<String>,
    /// Local service to forward to: PORT (on 127.0.0.1) or HOST:PORT.
    #[arg(short = 't', long, value_name = "[HOST:]PORT", value_parser = parse_service_addr)]
    pub target: Option<ServiceAddr>,
    /// Forwarding protocol.
    #[arg(short = 'p', long, value_enum)]
    pub protocol: Option<ProtocolArg>,
}

/// Service flags of the Access role.
#[derive(Args, Debug)]
pub struct AccessServiceArgs {
    /// Service name (lowercase letters, digits, '-'; must match on both sides).
    #[arg(short = 'n', long, value_parser = NonEmptyStringValueParser::new())]
    pub name: Option<String>,
    /// Local address to listen on: PORT (on 127.0.0.1) or HOST:PORT.
    #[arg(short = 'l', long, value_name = "[HOST:]PORT", value_parser = parse_service_addr)]
    pub listen: Option<ServiceAddr>,
    /// Serve NodeId (public identity, not authorization; protect the target service).
    #[arg(short = 'N', long, value_parser = NonEmptyStringValueParser::new())]
    pub node_id: Option<String>,
    /// Forwarding protocol.
    #[arg(short = 'p', long, value_enum)]
    pub protocol: Option<ProtocolArg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(host: Option<&str>, port: u16) -> ServiceAddr {
        ServiceAddr {
            host: host.map(str::to_owned),
            port: NonZeroU16::new(port).unwrap(),
        }
    }

    #[test]
    fn accepts_valid_forms() {
        assert_eq!(parse_service_addr("3000"), Ok(addr(None, 3000)));
        assert_eq!(
            parse_service_addr("10.0.0.5:3000"),
            Ok(addr(Some("10.0.0.5"), 3000))
        );
        assert_eq!(
            parse_service_addr("db.internal:5432"),
            Ok(addr(Some("db.internal"), 5432))
        );
        assert_eq!(
            parse_service_addr("[::1]:8080"),
            Ok(addr(Some("::1"), 8080))
        );
    }

    #[test]
    fn rejects_invalid_forms() {
        for bad in [
            "0",
            "70000",
            ":3000",
            "host:",
            "::1:8080",
            "not-an-address",
            "a b:1",
            "[::1]",
        ] {
            let err = parse_service_addr(bad).expect_err(bad);
            assert!(
                err.contains("port") || err.contains("address"),
                "{bad}: {err}"
            );
        }
    }
}
