//! Bidirectional byte-pipe between two async streams.
//!
//! Implements T-04. TCP is a byte stream, so for TCP tunneling we copy bytes
//! verbatim in both directions with [`tokio::io::copy_buf`] — no framing, no
//! transformation. The tunnel is transparent: the payload reaches the local
//! service byte-for-byte identical to what the peer sent.
//!
//! ## Half-close
//!
//! When one direction finishes (EOF on its read side) we [`AsyncWriteExt::shutdown`]
//! the matching write side so the peer learns the half-stream closed, then keep
//! waiting for the other direction to drain. This mirrors TCP half-close
//! semantics rather than tearing the whole connection down on the first EOF,
//! which matters for protocols (e.g. HTTP/1.1, postgres) that may finish
//! sending in one direction before the other.
//!
//! ## Interface
//!
//! The public entry is [`pipe_tcp_bidirectional`], which takes the local side
//! as a concrete [`TcpStream`] so it can use lock-free [`TcpStream::into_split`]
//! (no `BiLock` contention) plus 64 KiB read buffers. The copy/shutdown/join
//! core lives in the private [`pipe_halves`] helper; future stream types (a
//! TLS stream, a Unix socket) can plug into it without duplicating the
//! half-close semantics.

use anyhow::Result;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Copy bytes in both directions between a local [`TcpStream`] and a remote
/// read/write pair, using lock-free [`TcpStream::into_split`] and 64 KiB
/// copy buffers.
///
/// Splits the local side with [`TcpStream::into_split`] to obtain
/// `OwnedReadHalf` / `OwnedWriteHalf`. These are backed by `Arc<Mutex<…>>`
/// rather than `BiLock`, so concurrent reads and writes on the two halves
/// never contend — a real TCP socket is read/write-safe at the OS level.
///
/// Both copy directions use [`BufReader::with_capacity`]`(`[`64 * 1024`]`, …)`
/// together with [`tokio::io::copy_buf`] so that each direction enjoys a 64 KiB
/// internal buffer instead of the default 8 KiB, reducing syscall overhead on
/// large transfers.
///
/// Half-close semantics: when one direction finishes (EOF) the matching write
/// side is shut down, and the function returns once *both* directions have
/// completed.
pub(crate) async fn pipe_tcp_bidirectional<R, W>(local: TcpStream, remote: (R, W)) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (ri, wi) = remote;
    let (read_half, write_half) = local.into_split();

    // Wrap both read sides in a 64 KiB BufReader so copy_buf uses a
    // 64 KiB buffer instead of the default 8 KiB.
    let local_reader = BufReader::with_capacity(64 * 1024, read_half);
    let remote_reader = BufReader::with_capacity(64 * 1024, ri);

    // Two directions, each expressed as a (read, write) pair:
    //   local→remote: read from the local side, write to the remote write half
    //   remote→local: read from the remote side, write to the local write half
    // Grouping by direction (not by endpoint) makes the wiring impossible to
    // cross at the call site.
    pipe_halves((local_reader, wi), (remote_reader, write_half)).await
}

/// Drive two copy tasks concurrently and propagate both shutdowns, returning
/// once both directions are done.
///
/// Each argument is one direction's `(read, write)` pair — bytes flow from
/// that direction's read side to its write side. The two directions are
/// independent and run under a single [`tokio::join!`].
///
/// The half-close contract: each direction runs `copy_buf`, then unconditionally
/// `shutdown()`s its write side (so the peer sees FIN even when the copy
/// errored), then surfaces the copy error, or the shutdown error when the copy
/// succeeded. If both directions error, the first error wins.
///
/// The read sides take [`AsyncBufRead`] because [`tokio::io::copy_buf`]
/// requires it; callers wrap their streams in [`BufReader`] to pick the
/// buffer size.
///
/// Private so the half-close semantics have exactly one definition site. The
/// public [`pipe_tcp_bidirectional`] is the only caller today; a future
/// stream-type entry (TLS, Unix socket) would feed it different direction
/// pairs.
async fn pipe_halves<AR, AW, BR, BW>(
    (mut a_read, mut a_write): (AR, AW),
    (mut b_read, mut b_write): (BR, BW),
) -> Result<()>
where
    AR: AsyncBufRead + Unpin,
    AW: AsyncWrite + Unpin,
    BR: AsyncBufRead + Unpin,
    BW: AsyncWrite + Unpin,
{
    let (a, b) = tokio::join!(
        copy_then_half_close(&mut a_read, &mut a_write),
        copy_then_half_close(&mut b_read, &mut b_write),
    );
    a?;
    b?;
    Ok(())
}

/// One direction of [`pipe_halves`]: copy, then always attempt a clean
/// half-close so the peer sees FIN even when the copy errored. The copy error
/// wins; a shutdown error is returned only when the copy succeeded.
async fn copy_then_half_close<R, W>(read: &mut R, write: &mut W) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let copied = tokio::io::copy_buf(read, write).await;
    let closed = write.shutdown().await;
    copied?;
    closed
}

/// How long a completed pipe waits for the peer to acknowledge final stream
/// bytes. Technical implementation default, not configurable.
///
/// Kept below [`crate::shutdown::DRAIN_GRACE`] so that during shutdown a pipe
/// that completes early in the drain still gets its full acknowledgement wait
/// before the drain deadline aborts the task; a longer value would be moot.
pub(crate) const DELIVERY_ACK_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

const _: () = assert!(
    DELIVERY_ACK_GRACE.as_millis() < crate::shutdown::DRAIN_GRACE.as_millis(),
    "the delivery acknowledgement wait must fit inside the shutdown drain grace"
);

/// Outcome of waiting for the peer to acknowledge a finished send stream.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Delivery<E> {
    /// The peer acknowledged every byte including the FIN.
    Acknowledged,
    /// The peer sent STOP_SENDING with this code: it did not read all bytes.
    PeerStopped(iroh::endpoint::VarInt),
    /// No acknowledgement within the grace: delivery is unconfirmed, the peer
    /// may be lost. Bytes are never replayed.
    Unconfirmed,
    /// The connection failed before delivery could be confirmed.
    Lost(E),
}

/// Wait at most `grace` for `delivered` (a send stream's `stopped()` future).
///
/// QUIC shutdown only queues FIN, so pipe completion alone does not prove the
/// final bytes reached the peer. Callers map every non-[`Delivery::Acknowledged`]
/// outcome to a peer-loss style error or warning; a timeout is deliberately not
/// reported as success (AC-TCP-06), and nothing is retransmitted at this layer.
pub(crate) async fn await_delivery<F, E>(delivered: F, grace: std::time::Duration) -> Delivery<E>
where
    F: std::future::Future<Output = Result<Option<iroh::endpoint::VarInt>, E>>,
{
    match tokio::time::timeout(grace, delivered).await {
        Ok(Ok(None)) => Delivery::Acknowledged,
        Ok(Ok(Some(code))) => Delivery::PeerStopped(code),
        Ok(Err(error)) => Delivery::Lost(error),
        Err(_) => Delivery::Unconfirmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- await_delivery tests ----

    #[tokio::test]
    async fn await_delivery_reports_unconfirmed_when_no_ack_arrives() {
        let pending = std::future::pending::<Result<Option<iroh::endpoint::VarInt>, String>>();
        let outcome = await_delivery(pending, std::time::Duration::from_millis(30)).await;
        assert_eq!(outcome, Delivery::Unconfirmed);
    }

    #[tokio::test]
    async fn await_delivery_maps_ack_peer_stop_and_loss() {
        let grace = std::time::Duration::from_secs(5);
        let ok = std::future::ready(Ok::<_, String>(None));
        assert_eq!(await_delivery(ok, grace).await, Delivery::Acknowledged);
        let code = iroh::endpoint::VarInt::from_u32(42);
        let stopped = std::future::ready(Ok::<_, String>(Some(code)));
        assert_eq!(
            await_delivery(stopped, grace).await,
            Delivery::PeerStopped(code)
        );
        let lost = std::future::ready(Err::<Option<iroh::endpoint::VarInt>, _>("gone".to_string()));
        assert_eq!(
            await_delivery(lost, grace).await,
            Delivery::Lost("gone".to_string())
        );
    }
    use tokio::io::{duplex, AsyncReadExt};
    use tokio::net::TcpListener;

    // ---- pipe_tcp_bidirectional tests ----

    /// Helper: create a connected TCP pair bound to 127.0.0.1.
    /// Returns `(server, client)` — the caller picks which end is "local".
    async fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let server = listener.accept().await.unwrap().0;
        (server, client)
    }

    /// Wire a real TCP connection and a duplex stream together through
    /// `pipe_tcp_bidirectional` and confirm bytes flow in both directions
    /// without modification.
    ///
    /// Because `pipe_tcp_bidirectional` requires a concrete `TcpStream`
    /// (for `into_split`), the local side can't be an in-memory `duplex`;
    /// we create a loopback TCP pair bound to 127.0.0.1 instead.
    #[tokio::test]
    async fn tcp_copies_bytes_in_both_directions() {
        // Real TCP pair for the local side (pipe_tcp_bidirectional requires TcpStream).
        let (local, mut peer) = tcp_pair().await;
        // In-memory duplex for the remote side.
        let (remote_a, mut remote_b) = duplex(8 * 1024);
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_tcp_bidirectional(
            local,
            (remote_a_read, remote_a_write),
        ));

        // peer -> remote (peer writes to its TCP end, pipe copies local->remote).
        peer.write_all(b"hello-from-peer").await.unwrap();
        let mut got = [0u8; 15];
        remote_b.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello-from-peer");

        // remote -> peer (remote writes, pipe copies remote->local).
        remote_b.write_all(b"hello-from-remote").await.unwrap();
        let mut got = [0u8; 17];
        peer.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello-from-remote");

        // Close peer write side -> EOF propagates to remote read side.
        peer.shutdown().await.unwrap();
        let mut eof = [0u8; 1];
        let n = remote_b.read(&mut eof).await.unwrap();
        assert_eq!(n, 0, "expected EOF on remote read after peer shutdown");

        // Close remote write side -> EOF propagates to peer read side, which
        // lets the pipe task finish.
        let _ = remote_b.shutdown().await;

        pipe.await.unwrap().unwrap();
    }

    /// Half-close on the TCP peer side propagates as EOF through the pipe to
    /// the remote duplex read side, and data can still flow in the opposite
    /// direction after the half-close.
    #[tokio::test]
    async fn tcp_half_close_propagates_eof() {
        let (local, mut peer) = tcp_pair().await;
        let (remote_a, mut remote_b) = duplex(8 * 1024);
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_tcp_bidirectional(
            local,
            (remote_a_read, remote_a_write),
        ));

        // Shut down peer's write half immediately — no data sent in this direction.
        peer.shutdown().await.unwrap();

        // remote_b should see EOF on its read side (the pipe's local->remote copy
        // encounters EOF from the local TcpStream's read half, then shuts down
        // remote_a_write, propagating FIN to remote_b).
        let mut eof = [0u8; 1];
        let n = remote_b.read(&mut eof).await.unwrap();
        assert_eq!(
            n, 0,
            "expected EOF on remote after peer shutdown (half-close)"
        );

        // Data can still flow remote -> peer after the half-close.
        remote_b.write_all(b"final-data").await.unwrap();
        let mut got = [0u8; 10];
        peer.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"final-data");

        // Shut down remote write side so the pipe finishes.
        let _ = remote_b.shutdown().await;

        pipe.await.unwrap().unwrap();
    }

    /// A larger payload passes through byte-for-byte in both directions — a
    /// regression guard against accidental truncation or buffer issues in the
    /// 64 KiB `copy_buf` path used by `pipe_tcp_bidirectional`.
    #[tokio::test]
    async fn tcp_larger_payload_roundtrips_intact() {
        let (local, mut peer) = tcp_pair().await;
        let (remote_a, mut remote_b) = duplex(64 * 1024);
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_tcp_bidirectional(
            local,
            (remote_a_read, remote_a_write),
        ));

        // peer -> remote: 50 KiB payload.
        let payload_out: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();
        peer.write_all(&payload_out).await.unwrap();
        peer.shutdown().await.unwrap();
        let mut got_remote = vec![0u8; payload_out.len()];
        remote_b.read_exact(&mut got_remote).await.unwrap();
        assert_eq!(got_remote, payload_out);

        // remote -> peer: 50 KiB payload (different pattern).
        let payload_back: Vec<u8> = (0..50_000).map(|i| ((i + 7) % 251) as u8).collect();
        remote_b.write_all(&payload_back).await.unwrap();
        let _ = remote_b.shutdown().await;
        let mut got_peer = vec![0u8; payload_back.len()];
        peer.read_exact(&mut got_peer).await.unwrap();
        assert_eq!(got_peer, payload_back);

        pipe.await.unwrap().unwrap();
    }

    /// A writer whose copy succeeds but whose half-close fails.
    struct FailingShutdown;

    impl AsyncWrite for FailingShutdown {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()))
        }
    }

    /// A failed half-close after a successful copy is an error: the peer may
    /// never see FIN, so the channel did not complete cleanly.
    #[tokio::test]
    async fn half_close_failure_after_a_successful_copy_is_reported() {
        let empty: &[u8] = &[];
        let err = pipe_halves((empty, FailingShutdown), (empty, tokio::io::sink()))
            .await
            .expect_err("a failed shutdown must surface");
        assert!(
            err.chain().any(|c| c
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)),
            "{err:?}"
        );
    }

    /// The copy error wins over a half-close error in the same direction.
    #[tokio::test]
    async fn copy_failure_wins_over_a_failed_half_close() {
        let failing_read = tokio::io::BufReader::new(reset_reader());
        let err = pipe_halves(
            (failing_read, FailingShutdown),
            (&[][..], tokio::io::sink()),
        )
        .await
        .expect_err("a failed copy must surface");
        assert!(
            err.chain().any(|c| c
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionReset)),
            "{err:?}"
        );
    }

    /// A reader that fails its first read with `ConnectionReset`.
    fn reset_reader() -> impl tokio::io::AsyncRead + Unpin {
        struct Reset;
        impl tokio::io::AsyncRead for Reset {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()))
            }
        }
        Reset
    }
}
