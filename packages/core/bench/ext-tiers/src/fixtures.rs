//! The feeds both RSS tiers read: the connector's own test fixtures,
//! served on loopback with an ETag so a second poll is answered 304.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The fixtures, beside the vendored connector.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("rss/node/fixtures")
}

fn kind(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("json") => "application/feed+json",
        Some("html") => "text/html; charset=utf-8",
        Some("rdf") => "application/rdf+xml",
        _ => "application/xml",
    }
}

/// Serves [`dir`] on a loopback port until the runtime ends; answers its address.
pub async fn serve() -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let root = Arc::new(dir());
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(answer(stream, Arc::clone(&root)));
        }
    });
    Ok(addr)
}

async fn answer(mut stream: TcpStream, root: Arc<PathBuf>) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&raw);
    let path = head.split(' ').nth(1).unwrap_or("/");
    let name = path.trim_start_matches('/').split('?').next().unwrap_or_default();
    let asked = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(n, _)| n.eq_ignore_ascii_case("if-none-match"))
        .map(|(_, v)| v.trim().to_owned());
    let file = (!name.contains('/') && !name.is_empty()).then(|| root.join(name));
    let body = match file {
        Some(f) => tokio::fs::read(f).await.ok(),
        None => None,
    };
    let reply = match body {
        Some(body) => {
            let etag = format!("\"{}-{}\"", name, body.len());
            if asked.as_deref() == Some(etag.as_str()) {
                format!("HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nConnection: close\r\n\r\n").into_bytes()
            } else {
                let mut r = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n\r\n",
                    kind(name),
                    body.len()
                )
                .into_bytes();
                r.extend_from_slice(&body);
                r
            }
        }
        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
    };
    let _ = stream.write_all(&reply).await;
    let _ = stream.shutdown().await;
}
