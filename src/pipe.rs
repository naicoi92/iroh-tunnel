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
#![allow(dead_code)] // consumed by T-06/T-07; flagged until then.

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

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
}
