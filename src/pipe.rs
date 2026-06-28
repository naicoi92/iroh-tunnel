//! Bidirectional byte-pipe between two async streams.
//!
//! Implements T-04. TCP is a byte stream, so for TCP tunneling we copy bytes
//! verbatim in both directions with [`tokio::io::copy`] — no framing, no
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
//! ## Generics
//!
//! The function is generic over an `AsyncRead + AsyncWrite` "local" side and an
//! arbitrary remote pair (`(R, W)`), rather than an iroh `BidiStream`, because
//! iroh 1.0 exposes a bidirectional QUIC stream as a `(RecvStream, SendStream)`
//! tuple via [`Connection::accept_bi`] / [`Connection::open_bi`]. Accepting the
//! pair directly keeps this module free of iroh types and trivially unit-testable
//! with plain in-memory streams. The serve/access handlers (T-06/T-07) pass
//! `(RecvStream, SendStream)` straight through.
//!
//! [`Connection::accept_bi`]: iroh::endpoint::Connection::accept_bi
//! [`Connection::open_bi`]: iroh::endpoint::Connection::open_bi
//!
//! ## UDP framing (T-10)
//!
//! UDP carries datagrams with boundaries, but an iroh bidirectional stream is a
//! byte stream — so to tunnel UDP transparently we length-prefix each datagram
//! with a big-endian `u32` and let the peer re-slice on the other side
//! (`[len][payload]`). The payload bytes are untouched. See [`encode_frame`] /
//! [`decode_frame`] for the codec and [`pipe_udp`] for the framed pipe.
#![allow(dead_code)] // TCP pipe consumed by T-06/T-07; UDP codec/pipe by future UDP handlers.

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest UDP datagram (header + payload) we will accept in a frame. Guards
/// against a malicious/garbled length prefix causing a huge allocation.
const MAX_DATAGRAM: usize = 65535;

/// Copy bytes in both directions between a local stream and a remote read/write
/// pair.
///
/// Returns once *both* directions have completed (either normally via EOF or via
/// a half-close). An error on either direction is propagated after the other
/// direction has had a chance to flush its shutdown; if both error, the first
/// error wins.
///
/// `local` and `remote`'s halves are all split internally via
/// [`tokio::io::split`], so each direction runs on its own `JoinHandle`-free
/// task driven by [`tokio::join!`].
pub async fn pipe_bidirectional<L, R, W>(local: L, remote: (R, W)) -> Result<()>
where
    L: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (mut ri, mut wi) = remote;
    let (mut li, mut lo) = tokio::io::split(local);

    // local -> remote
    let c2r = async {
        let res = tokio::io::copy(&mut li, &mut wi).await;
        // Always attempt a clean half-close so the peer sees FIN, even when the
        // copy errored. The original error (if any) is what we return.
        let _ = wi.shutdown().await;
        let _ = res?;
        Ok::<_, std::io::Error>(())
    };

    // remote -> local
    let r2c = async {
        let res = tokio::io::copy(&mut ri, &mut lo).await;
        let _ = lo.shutdown().await;
        let _ = res?;
        Ok::<_, std::io::Error>(())
    };

    let (a, b) = tokio::join!(c2r, r2c);
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
    use tokio::io::{duplex, AsyncReadExt, DuplexStream};

    /// Wire two duplex streams together through `pipe_bidirectional` and
    /// confirm bytes flow in both directions without modification.
    ///
    /// The pipe owns `local_a` and the split `(remote_a_read, remote_a_write)`
    /// pair. We drive the outer ends `local_b` and `remote_b` from the test.
    #[tokio::test]
    async fn copies_bytes_in_both_directions() {
        let (local_a, mut local_b) = duplex(8 * 1024);
        let (remote_a, mut remote_b) = duplex(8 * 1024);

        // remote_a is the end the pipe talks to; split it into its read/write
        // halves for the (R, W) tuple.
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_bidirectional(local_a, (remote_a_read, remote_a_write)));

        // local -> remote
        local_b.write_all(b"hello-from-local").await.unwrap();
        let mut got = [0u8; 16];
        remote_b.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello-from-local");

        // remote -> local
        remote_b.write_all(b"hello-from-remote").await.unwrap();
        let mut got = [0u8; 17];
        local_b.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"hello-from-remote");

        // Close local write side -> EOF propagates to remote read side.
        local_b.shutdown().await.unwrap();
        let mut eof = [0u8; 1];
        let n = remote_b.read(&mut eof).await.unwrap();
        assert_eq!(n, 0, "expected EOF on remote read after local shutdown");

        // Close remote write side -> EOF propagates to local read side, which
        // lets the pipe task finish.
        let _ = remote_b.shutdown().await;

        pipe.await.unwrap().unwrap();
    }

    /// A larger payload passes through byte-for-byte, in each direction, on its
    /// own task — a regression guard against accidental truncation or framing.
    #[tokio::test]
    async fn larger_payload_roundtrips_intact_local_to_remote() {
        let (local_a, local_b) = duplex(64 * 1024);
        let (remote_a, mut remote_b) = duplex(64 * 1024);
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_bidirectional(local_a, (remote_a_read, remote_a_write)));

        let payload: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();

        let send = {
            let payload = payload.clone();
            tokio::spawn(async move {
                let mut local_b = local_b;
                local_b.write_all(&payload).await.unwrap();
                local_b.shutdown().await.unwrap();
            })
        };
        let mut got = vec![0u8; payload.len()];
        remote_b.read_exact(&mut got).await.unwrap();
        assert_eq!(got, payload);
        send.await.unwrap();

        // Drain remote_b so the pipe's remote->local copy sees EOF and exits.
        drop(remote_b);
        pipe.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn larger_payload_roundtrips_intact_remote_to_local() {
        let (local_a, local_b) = duplex(64 * 1024);
        let (remote_a, remote_b) = duplex(64 * 1024);
        let (remote_a_read, remote_a_write) = tokio::io::split(remote_a);
        let pipe = tokio::spawn(pipe_bidirectional(local_a, (remote_a_read, remote_a_write)));

        let payload: Vec<u8> = (0..50_000).map(|i| (i % 251) as u8).collect();

        let send = {
            let payload = payload.clone();
            tokio::spawn(async move {
                let mut remote_b = remote_b;
                remote_b.write_all(&payload).await.unwrap();
                let _ = remote_b.shutdown().await;
            })
        };
        let mut got = vec![0u8; payload.len()];
        let mut local_b = local_b;
        local_b.read_exact(&mut got).await.unwrap();
        assert_eq!(got, payload);
        send.await.unwrap();

        drop(local_b);
        pipe.await.unwrap().unwrap();
    }

    /// Sanity-check the unused-parameter guard: the type system requires both
    /// remote halves to be `Unpin`. This compiles iff the bounds are satisfiable
    /// for tokio's split halves (regression guard against tightening them too
    /// far in a refactor).
    #[allow(dead_code)]
    fn _bounds_are_satisfiable(
        local: DuplexStream,
        remote: (
            tokio::io::ReadHalf<DuplexStream>,
            tokio::io::WriteHalf<DuplexStream>,
        ),
    ) {
        // If this stops compiling, pipe_bidirectional's bounds drifted.
        fn assert_pipe<L, R, W>(_: L, _: (R, W))
        where
            L: AsyncRead + AsyncWrite + Unpin,
            R: AsyncRead + Unpin,
            W: AsyncWrite + Unpin,
        {
        }
        let (r, w) = remote;
        assert_pipe(local, (r, w));
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
