# Multiplexing per connection + dual-ALPN negotiation — Design

Ngày: 2026-08-19
Trạng thái: Đã duyệt (user)
PR target: `feat/multiplex-dual-alpn` → `main`

## Động lực

Hiện trạng 1:1: mỗi local TCP connection phía access mở một iroh connection riêng
(relay session + QUIC/TLS handshake riêng), và serve chỉ accept đúng 1 bidi stream
trên mỗi connection (`handle_connection` gọi `accept_bi` một lần rồi return).
Workload nhiều kênh song song đến cùng serve node trả handshake N lần.

QUIC/iroh hỗ trợ multi-stream native trên 1 connection — giới hạn hoàn toàn là
app-level. Mục tiêu: **1 connection dài hạn giữa access ↔ serve cho mỗi service;
mỗi local TCP connection = 1 QUIC bidi stream**.

## Facts đã verify từ source (iroh 1.0.0, pin trong Cargo.lock)

- `Endpoint::connect()` **không reuse** connection — mỗi call tạo QUIC connection
  mới (comment "idempotent" cũ trong `access.rs` sai, được sửa kèm PR này).
- ALPN không khớp → handshake fail nhanh, lỗi surface là
  `ConnectError::Connection { ConnectionError::ConnectionClosed(..) }`.
  Lỗi network (TimedOut, unreachable…) là class khác → không dùng làm tín hiệu
  fallback.
- `QuicTransportConfig::builder()` khởi tạo từ iroh defaults (keepalive 5s,
  path idle 15s, multipath…) — append thêm `max_concurrent_bidi_streams(VarInt)`
  và gán qua `Endpoint::builder().transport_config(cfg)` không mất defaults.
- noq default `max_idle_timeout` = 30s nhưng keepalive 5s (iroh default) giữ
  connection sống qua idle vô hạn khi path còn sống.

## Quyết định thiết kế

### 1. Dual-ALPN (protocol negotiation có chủ đích)

- Serve mới đăng ký **cả hai** ALPN cho mỗi service:
  - legacy: `iroh-tunnel/{name}` (giữ nguyên, tương thích hoàn toàn)
  - multi: `iroh-tunnel/{name}/multi` (suffix `/multi` — tự diễn giải, grep được
    trong packet capture; service name `[a-z0-9-]` không chứa `/` nên parse
    không nhập nhằng)
- Access mới (mode `auto`) dial multi trước. Gặp serve cũ → ALPN không khớp →
  connection bị từ chối thẳng tại handshake (**fail nhanh, không treo**) → tự
  fallback sang legacy ALPN + chế độ connection-mỗi-kênh (behavior hiện tại).
- Ma trận tương thích: access cũ ↔ serve mới = như cũ (serve mới vẫn phục vụ
  stream đầu y như cũ trên legacy ALPN — thực tế serve mới phục vụ mọi stream
  trên legacy ALPN, access cũ chỉ cần stream đầu).

### 2. Serve — accept loop per connection

- `handle_connection` → `loop { conn.accept_bi() }`: mỗi stream →
  `TcpStream::connect(local)` riêng → spawn pipe task riêng. Connection phục vụ
  mọi stream cho tới khi `accept_bi` lỗi (connection đóng).
- Demux: cả legacy + multi ALPN map về cùng `local_addr` của service.
- Error semantics per-stream độc lập: 1 stream fail/EOF không ảnh hưởng stream
  khác; local connect fail → drop stream halves (QUIC reset stream đó).

### 3. Access — connection dài hạn + open_bi per kênh

- `AccessService` thêm field `multiplex: MultiplexMode` (`auto|on|off`,
  default `auto`, serde lowercase). Chỉ có ý nghĩa với TCP; UDP path không đổi.
- Multi path (negotiated): mỗi service giữ shared `Mutex<Option<Connection>>`;
  mỗi local TCP conn accepted → lấy/dial connection (multi ALPN, **1 attempt
  không retry-forever**) → `open_bi` → pipe. Local TCP đóng → stream đóng,
  connection GIỮ.
- `open_bi` lỗi connection-level → drop cache, redial 1 lần, retry 1 lần; vẫn
  lỗi → trả lỗi cho kênh đó (semantics port-forward: nhận EOF).
- Fallback: multi dial bị reject (class `ConnectionClosed`) → đánh dấu service
  `fallback=legacy` (sticky trong đời shared state); các kênh đi legacy
  per-connection path. Reset cờ khi legacy conn cuối cùng đóng → chu kỳ dial kế
  re-probe multi (serve upgrade thì tự lên). `on` = không fallback, lỗi rõ.
  `off` = luôn legacy.
- Keepalive: set tường minh `keep_alive_interval(5s)` cho access multi mode +
  serve — chống drift default iroh, tự documentation.

### 4. Transport config

- Serve + access: `max_concurrent_bidi_streams = 256` tường minh (headroom
  tuning, không phải requirement). Docs: worst-case memory ∝
  `max_concurrent_bidi_streams × stream_receive_window`; hết slot → `open_bi`
  bị flow-control block chờ stream khác đóng.

### 5. Status file

- `active_connections` (serve) = số **streams active** (đang pipe) per service:
  `Arc<AtomicU64>` tăng/giảm trong stream task; task nền flush status.json mỗi
  5s, chỉ ghi khi giá trị đổi (tránh disk churn). Field name giữ nguyên cho
  compat schema, nghĩa mới ghi rõ docs.

### 6. Ngoài scope

- UDP path: không đụng.
- Không thêm framing/mux tự chế — chỉ QUIC stream native.

## Testing

- Unit: proto roundtrip 2 ALPN forms; config parse 3 mode + default; backoff.
- Integration (`tests/serve_access_tunnel.rs`, giữ convention `#[ignore]` +
  n0 relay):
  1. N stream song song trên 1 connection; đóng 1 stream, stream khác sống.
  2. Compat: access cấu hình `multiplex = "off"` ↔ serve mới — behavior như cũ.
  3. Fallback: serve chỉ register legacy ALPN (giả serve cũ) → access `auto`
     tự fallback, kênh hoạt động, không treo.
  4. N local TCP đồng thời đi trên 1 iroh connection (verify qua counter).
  5. Keepalive: idle > 30s rồi mở stream mới vẫn hoạt động.

## Release

- Version 0.2.0 (minor, non-breaking, tự tương thích ngược).
- CHANGELOG.md (mới): 2 mode, negotiation semantics, default, trade-off.
- README mục "Multiplexing": khi nào lợi (nhiều kênh song song到一个 serve
  node), khi nào tắt (muốn isolation giữa kênh), memory/blocking trade-off.
