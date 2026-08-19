# Multiplex per connection + dual-ALPN — Implementation Plan

> **For agentic workers:** Executed inline (executing-plans) trong session này.
> Spec: `docs/superpowers/specs/2026-08-19-multiplex-dual-alpn-design.md`

**Goal:** 1 iroh connection dài hạn giữa access ↔ serve cho mỗi service; mỗi local
TCP connection = 1 QUIC bidi stream; negotiation dual-ALPN fail-fast fallback.

**Worktree:** `../iroh-tunnel-multiplex`, branch `feat/multiplex-dual-alpn` từ
`origin/main`.

## Global Constraints

- iroh pin 1.0.0 (Cargo.lock) — chỉ dùng API có trong bản này.
- Ngôn ngữ code/comment/docs: tiếng Anh (repo convention), tiếng Việt cho
  chat/PR description.
- UDP path không đụng. Không framing/mux tự chế.
- Tests integration giữ convention `#[ignore]` + n0 relay.
- Version 0.2.0, CHANGELOG.md mới, README mục Multiplexing.

## Tasks

### Task 1: proto — ALPN variant

- `src/proto.rs`: thêm `MULTIPLEX_SUFFIX = "/multi"`, `multiplex_alpn_for(name)`,
  `name_from_alpn` nhận diện cả 2 (strip suffix nếu có). Unit tests: roundtrip,
  suffix strip, suffix không cho name có `/` (name regex đã chặn).

### Task 2: config — MultiplexMode

- `src/config.rs`: `enum MultiplexMode { Auto, On, Off }`, serde lowercase,
  Default = Auto. `AccessService.multiplex: MultiplexMode` với
  `#[serde(default)]`. Validation: chấp nhận cho mọi protocol (chỉ ý nghĩa tcp;
  docs ghi rõ). Unit tests: parse 3 giá trị + default + TOML không có field.

### Task 3: endpoint — transport config

- `src/endpoint.rs`: `create_serve_endpoint`/`create_access_endpoint` build
  `QuicTransportConfig::builder().max_concurrent_bidi_streams(256u32.into())
  .keep_alive_interval(Duration::from_secs(5)).build()` → gán
  `builder.transport_config(cfg)`. Const `MAX_CONCURRENT_BIDI_STREAMS: u32 = 256`
  + doc memory trade-off.

### Task 4: serve — per-connection accept loop + stream counter

- `src/serve.rs`:
  - `build_endpoint`: register cả `alpn_for` + `multiplex_alpn_for` cho mọi
    service.
  - `run_loop`: `local_addrs` map cả 2 ALPN → cùng local_addr; status services
    giữ `Arc<AtomicU64>` counter; spawn flush task 5s chỉ-đổi; abort cùng accept
    task khi shutdown.
  - `accept_loop`: log `mode=legacy|multi` theo suffix.
  - `handle_connection` → loop `accept_bi`, mỗi stream: counter += 1 →
    `TcpStream::connect` → spawn `pipe_tcp_bidirectional` → counter -= 1 khi
    xong. `accept_bi` Err → connection đóng, log + return.

### Task 5: access — multi mode + fallback

- `src/access.rs` + `src/role_run.rs`:
  - `listen_loop` nhận `multiplex: MultiplexMode`. `off` → path hiện tại
    (per-channel `connect_with_retry` legacy, giữ nguyên).
  - `auto|on`: shared `ServiceConn` (`Mutex<Option<Connection>>` + fallback cờ).
    Local conn: lấy conn (nếu none → dial multi 1 lần; reject class
    `ConnectError::Connection{ConnectionClosed}` → `on` thì lỗi, `auto` thì set
    fallback + đi legacy path cho kênh) → `open_bi`; lỗi conn-level → drop cache
    + redial 1 lần + retry 1 lần → pipe.
  - Fallback cờ reset khi shared state hết conn legacy reference → chu kỳ sau
    re-probe multi.
  - Sửa comment sai về connect-idempotency.
- Unit test: classify `is_alpn_rejected` (dựng error giả theo variant).

### Task 6: integration tests

- `tests/serve_access_tunnel.rs` mở rộng 5 test theo spec (multi N-stream,
  compat off, fallback legacy-only serve, counter verify, keepalive idle 35s).
- Helper chung tách biệt; serve "giả cũ" = endpoint register đúng 1 legacy ALPN.

### Task 7: docs + version

- README mục "Multiplexing"; CHANGELOG.md 0.2.0; Cargo.toml 0.2.0; examples
  `access.toml` thêm ví dụ `multiplex`.

### Task 8: verify + PR

- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` (unit + doc).
- Chạy integration `--ignored` nếu network cho phép; nếu không, ghi rõ trong PR.
- Push branch, mở PR (description tiếng Việt), CI xanh.
