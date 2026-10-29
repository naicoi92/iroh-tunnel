# UDP transport research

This note informs the planned UDP forwarding design. It does not establish benchmark results or an implementation decision.

## Findings

- [Iroh's QUIC guide](https://docs.iroh.computer/protocols/using-quic) describes QUIC over UDP as the default, with NAT traversal and relay fallback handled by Iroh. Application protocols still need to choose how to exchange data; streams are reliable and ordered within each stream.
- [Iroh 1.4.1 `Connection`](https://docs.rs/iroh/1.4.1/iroh/endpoint/struct.Connection.html) exposes unreliable, unordered application datagrams. Their payload must fit in one QUIC packet and respect the peer's limit. The available maximum may change with path MTU and can be unavailable if datagrams are disabled or unsupported.
- Iroh 1.4.1 also exposes batched datagram send and receive operations; its documentation describes reduced per-datagram overhead for bursts. Measure their effect for this application rather than assuming a performance gain.
- [RFC 9221 §5–5.4](https://www.rfc-editor.org/rfc/rfc9221.html#section-5) specifies that QUIC DATAGRAM frames cannot be fragmented at the QUIC layer, share congestion control, and require application-defined identifiers to multiplex flows. Loss or congestion can result in dropped datagrams.
- The existing `src/pipe.rs` frames datagrams over byte streams, but the repository README states that UDP is not wired into the run path. Reliable ordered streams change latency and delivery behavior compared with the requested best-effort UDP contract.

## Design implications

QUIC DATAGRAM is a candidate for best-effort forwarding; the application must identify client flows and decide how to handle payloads exceeding the currently available datagram size. Application-layer fragmentation and bounded reassembly are candidates, not guarantees of delivery. Rejecting oversized payloads is simpler but reduces compatibility. A reliable stream fallback would change delivery semantics and must not be presented as equivalent without an explicit decision.

There is no measured evidence here that one option offers the best performance for this project. Compare latency, loss, throughput and memory under direct and relay paths, multiple clients, variable MTU and packet sizes, including congestion and disconnection, before committing to a transport strategy or numeric limits.
