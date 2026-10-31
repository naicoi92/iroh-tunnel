//! Local HTTP server standing in for GitHub Releases, shared by the installer
//! tests (#93).
use anyhow::Result;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Serves a directory over HTTP; `/latest/download/x` and `/download/vN/x`
/// both map to files below `root`, like GitHub Releases.
pub struct ReleaseServer {
    pub url: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ReleaseServer {
    pub fn start(root: PathBuf, version: &str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/releases", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let version = version.to_owned();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let _ = serve_one(stream, &root, &version);
                }
            }
        });
        Ok(Self {
            url,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ReleaseServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(addr) = self.url.strip_prefix("http://") {
            let _ = TcpStream::connect(addr.split('/').next().unwrap_or(addr));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve_one(mut stream: TcpStream, root: &Path, version: &str) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf)?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .trim_start_matches("/releases/")
        .to_owned();
    // latest/download/<file> and download/v<ver>/<file> -> <root>/<ver>/<file>
    let file = if let Some(rest) = path.strip_prefix("latest/download/") {
        root.join(version).join(rest)
    } else if let Some(rest) = path.strip_prefix("download/v") {
        root.join(rest)
    } else {
        root.join("missing")
    };
    match std::fs::read(&file) {
        Ok(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        Err(_) => {
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
    }
    Ok(())
}
