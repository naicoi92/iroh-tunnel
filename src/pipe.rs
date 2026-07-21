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
//!
//! ## UDP framing (T-10)
//!
//! UDP carries datagrams with boundaries, but an iroh bidirectional stream is a
//! byte stream — so to tunnel UDP transparently we length-prefix each datagram
//! with a big-endian `u32` and let the peer re-slice on the other side
//! (`[len][payload]`). The payload bytes are untouched. See [`encode_frame`] /
//! [`decode_frame`] for the codec and [`pipe_udp`] for the framed pipe.
#![allow(dead_code)] // UDP codec/pipe await future UDP handlers (T-10).

use anyhow::Result;
use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// Largest UDP datagram (header + payload) we will accept in a frame. Guards
/// against a malicious/garbled length prefix causing a huge allocation.
const MAX_DATAGRAM: usize = 65535;

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
pub async fn pipe_tcp_bidirectional<R, W>(local: TcpStream, remote: (R, W)) -> Result<()>
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
/// errored), then surfaces the original copy result. If both directions error,
/// the first error wins.
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
    // direction A: a_read → a_write
    let dir_a = async {
        let res = tokio::io::copy_buf(&mut a_read, &mut a_write).await;
        // Always attempt a clean half-close so the peer sees FIN, even when the
        // copy errored. The original error (if any) is what we return.
        let _ = a_write.shutdown().await;
        let _ = res?;
        Ok::<_, std::io::Error>(())
    };

    // direction B: b_read → b_write
    let dir_b = async {
        let res = tokio::io::copy_buf(&mut b_read, &mut b_write).await;
        let _ = b_write.shutdown().await;
        let _ = res?;
        Ok::<_, std::io::Error>(())
    };

    let (a, b) = tokio::join!(dir_a, dir_b);
    a.map_err(anyhow::Error::from)?;
    b.map_err(anyhow::Error::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UDP framing codec (T-10)
// ---------------------------------------------------------------------------

/// Append one UDP datagram to `buf` as a length-prefixed frame: `[u32 BE len][payload]`.
///
/// The payload is copied verbatim — no transformation — so the tunnel stays
/// transparent; the framing only preserves datagram boundaries across the byte
/// stream. `buf` is appended to (not cleared) so callers can batch.
pub fn encode_frame(buf: &mut Vec<u8>, payload: &[u8]) {
    let len = payload.len() as u32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(payload);
}

/// Read one length-prefixed frame from `r` and return its payload.
///
/// Returns the original datagram bytes. Bails if the decoded length exceeds
/// [`MAX_DATAGRAM`] (guards against a malicious/corrupt length prefix) or if the
/// stream ends mid-frame.
pub async fn decode_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_DATAGRAM {
        anyhow::bail!("datagram too large: {len}");
    }
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Pipe a local UDP socket against a remote framed byte-stream pair.
///
/// Each received datagram is [`encode_frame`]d before being written to the
/// remote; each [`decode_frame`]d payload is sent as one datagram on the local
/// socket. Returns when either direction ends (the other is dropped).
///
/// Note: the spec sample takes an `iroh::endpoint::BidiStream`, but iroh 1.0 has
/// no `BidiStream` type — a bidirectional QUIC stream is a `(RecvStream,
/// SendStream)` tuple (see the module-level note for the TCP pipe). We accept
/// that pair directly, matching how `pipe_bidirectional` already works; the
/// framing contract is unchanged.
pub async fn pipe_udp<R, W>(
    local: tokio::net::UdpSocket,
    mut remote_read: R,
    mut remote_write: W,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // local -> remote: each datagram becomes a frame.
    let c2r = async {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        let mut frame = Vec::new();
        while let Ok(n) = local.recv(&mut buf).await {
            frame.clear();
            encode_frame(&mut frame, &buf[..n]);
            if remote_write.write_all(&frame).await.is_err() {
                break;
            }
        }
    };

    // remote -> local: each frame becomes a datagram.
    let r2c = async {
        while let Ok(payload) = decode_frame(&mut remote_read).await {
            let _ = local.send(&payload).await;
        }
    };

    tokio::select! {
        _ = c2r => {},
        _ = r2c => {},
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    /// Unlike the `pipe_bidirectional` tests which can use `duplex` for the
    /// local side, `pipe_tcp_bidirectional` requires a concrete `TcpStream`
    /// (for `into_split`), so we create a loopback TCP pair.
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

    // ---- UDP framing codec (T-10) ----

    /// encode_frame + decode_frame round-trip preserves the datagram payload
    /// exactly and re-establishes its boundary. This is the primary T-10
    /// verify gate (Page 05 v3 §5.3).
    #[tokio::test]
    async fn frame_codec_roundtrips_one_datagram() {
        let payload = b"hello-udp-datagram";
        let mut frame = Vec::new();
        encode_frame(&mut frame, payload);
        // Frame = 4-byte BE length + payload, nothing more.
        assert_eq!(frame.len(), 4 + payload.len());

        let mut cursor = std::io::Cursor::new(frame);
        let decoded = decode_frame(&mut cursor).await.unwrap();
        assert_eq!(decoded, payload);
    }

    /// Multiple datagrams framed back-to-back decode independently, proving the
    /// length prefix preserves boundaries in the byte stream.
    #[tokio::test]
    async fn frame_codec_preserves_multiple_boundaries() {
        let dgrams: &[&[u8]] = &[b"one", b"", b"two-two", b"three-three-three"];
        let mut frame = Vec::new();
        for d in dgrams {
            encode_frame(&mut frame, d);
        }

        let mut cursor = std::io::Cursor::new(frame);
        for expected in dgrams {
            let decoded = decode_frame(&mut cursor).await.unwrap();
            assert_eq!(decoded.as_slice(), *expected);
        }
        // No trailing bytes — boundaries line up exactly.
        assert_eq!(cursor.position() as usize, cursor.get_ref().len());
    }

    /// A length prefix claiming more than MAX_DATAGRAM is rejected rather than
    /// causing a huge allocation.
    #[tokio::test]
    async fn frame_codec_rejects_oversized_length() {
        let mut frame = Vec::new();
        // Spoof a length one byte over the cap.
        let oversize = (MAX_DATAGRAM as u32 + 1).to_be_bytes();
        frame.extend_from_slice(&oversize);
        let mut cursor = std::io::Cursor::new(frame);
        let err = decode_frame(&mut cursor).await.unwrap_err().to_string();
        assert!(err.contains("too large"), "unexpected error: {err}");
    }
}
