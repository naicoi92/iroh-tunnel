# Multiplexing per connection — Design (single ALPN, serve-first rollout)

Ngày: 2026-08-19
Trạng thái: Đã duyệt (user) — **quyết định owner chốt: KHÔNG đổi ALPN, KHÔNG
đàm phán phiên bản. Rollout: upgrade serve trước, access sau.**
PR: #48 (`feat/multiplex-dual-alpn` → `main`)

> **Lịch sử thiết kế:** brief đầu tiên đề xuất dual-ALPN negotiation
> (`/multi` variant + fail-fast fallback). Một bản implement dual-ALPN đầy
> đủ (offer-both RFC 7301) đã được làm và test 5/5 xanh trên cùng PR, nhưng
> owner chốt đơn giản hóa: bỏ ALPN variant, bỏ negotiation, quản lý
> compatibility bằng thứ tự rollout. Các facts probe (giữ lại bên dưới làm
> tham khảo) vẫn đúng cho iroh 1.0.0.

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
- **Facts probe (tham khảo, không còn dùng):** ALPN-không-hợp bị iroh 1.0
  drop im lặng → client chờ 30s timeout (không alert); client offer danh
  sách không khớp hoàn toàn mới nhận `NoApplicationProtocol` 0x178 ~125ms.
  Với single-ALPN điều này không còn liên quan: mọi connection dùng
  `alpn_for(name)`.
- `QuicTransportConfig::builder()` khởi tạo từ iroh defaults (keepalive 5s,
  path idle 15s, multipath…) — append thêm `max_concurrent_bidi_streams(VarInt)`
  và gán qua `Endpoint::builder().transport_config(cfg)` không mất defaults.
- noq default `max_idle_timeout` = 30s nhưng keepalive 5s (iroh default) giữ
  connection sống qua idle vô hạn khi path còn sống.

## Quyết định thiết kế

### 1. Single ALPN + rollout contract

- ALPN giữ nguyên `iroh-tunnel/{name}` cho cả hai mode — KHÔNG negotiation.
- Ma trận tương thích quản bằng **thứ tự rollout serve-trước**:
  - access cũ ↔ serve mới: như cũ (serve mới vẫn phục vụ stream đầu y như
    cũ; thực tế phục vụ mọi stream).
  - access mới `multiplex = true` ↔ serve cũ: **không được hỗ trợ** (stream
    #2 không ai accept, treo tới khi conn đóng/idle timeout) — tránh bằng
    rollout đúng thứ tự; nếu buộc phải chạy thì `multiplex = false`.
  - access mới `multiplex = false`: behavior pre-0.2.0 nguyên vẹn với mọi
    serve.

### 2. Serve — accept loop per connection

- `handle_connection` → `loop { conn.accept_bi() }`: mỗi stream →
  `TcpStream::connect(local)` riêng → spawn pipe task riêng. Connection phục vụ
  mọi stream cho tới khi `accept_bi` lỗi (connection đóng).
- Demux: cả legacy + multi ALPN map về cùng `local_addr` của service.
- Error semantics per-stream độc lập: 1 stream fail/EOF không ảnh hưởng stream
  khác; local connect fail → drop stream halves (QUIC reset stream đó).

### 3. Access — connection dài hạn + open_bi per kênh

- `AccessService` thêm field `multiplex: bool` (default `true`, serde
  default function). Chỉ có ý nghĩa với TCP; UDP path không đổi.
- Multiplex path: mỗi service giữ shared `Mutex<Option<Connection>>`; mỗi
  local TCP conn accepted → `get_or_dial` (tái dùng `connect_with_retry`,
  lock giữ xuyên suốt dial để N kênh đồng thời chỉ tạo 1 connection) →
  `open_bi` → pipe. Local TCP đóng → stream đóng, connection GIỮ. `open_bi`
  lỗi conn-level → drop cache + redial 1 lần + retry 1 lần; vẫn lỗi → kênh
  nhận lỗi (semantics port-forward).
- `multiplex = false`: per-channel `connect_with_retry` như cũ.

### 4. Transport config

- (Chỉnh theo feedback owner sau review PR #48:) KHÔNG hardcode limit —
  mặc định giữ noq default `max_concurrent_bidi_streams = 100`; override qua
  config `[node] max_concurrent_streams` (Option<u32>, validate ≥ 1, cả hai
  role). Docs: worst-case memory ∝ `max_concurrent_streams ×
  stream_receive_window`; hết slot → `open_bi` bị flow-control block chờ
  stream khác đóng.

### 5. Status file

- `active_connections` (serve) = số **streams active** (đang pipe) per service:
  `Arc<AtomicU64>` tăng/giảm trong stream task; task nền flush status.json mỗi
  5s, chỉ ghi khi giá trị đổi (tránh disk churn). Field name giữ nguyên cho
  compat schema, nghĩa mới ghi rõ docs.

### 6. Ngoài scope

- UDP path: không đụng.
- Không thêm framing/mux tự chế — chỉ QUIC stream native.


## Testing

- Unit: config parse bool `multiplex` + default; backoff; proto như cũ.
- Integration (`tests/serve_access_tunnel.rs`, giữ convention `#[ignore]` +
  n0 relay):
  1. `serve_access_tunnel_roundtrips_bytes` — single-channel legacy pin
     (access cũ ↔ serve mới compat, đường legacy tự nhiên).
  2. N stream song song trên 1 connection; đóng 1 stream, stream khác sống.
  3. `multiplex = false`: 2 kênh tuần tự = 2 connection riêng.
  4. Keepalive: idle 60s (2× idle timeout 30s) rồi mở stream mới vẫn cùng
     connection, kênh cũ còn sống.

## Release

- Version 0.2.0 (minor — non-breaking với thứ tự rollout đúng).
- CHANGELOG.md: ma trận tương thích + yêu cầu rollout serve-trước, default,
  trade-off.
- README mục "Multiplexing": khi nào lợi (nhiều kênh song song đến một serve
  node), khi nào tắt (isolation giữa kênh, hoặc serve chưa upgrade),
  memory/blocking trade-off.
