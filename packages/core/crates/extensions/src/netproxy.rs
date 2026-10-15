//! The network a sandboxed program reaches: a proxy on loopback that lets a
//! program through only to the hosts its manifest declares.
//!
//! A program under the sandbox profile may connect to nothing but this
//! proxy's port. It is started with `HTTPS_PROXY` and `HTTP_PROXY` naming the
//! proxy with a grant's token as the password, which every runtime that
//! honours those variables sends back as `Proxy-Authorization`. The proxy
//! tunnels `CONNECT` and forwards plain `http://` requests to the granted
//! hosts, and refuses the rest with 403, naming the host. A tunnel is TLS end
//! to end, so the program's own credentials travel inside it untouched.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tracing::info;

use crate::gate::declared_host;

/// The longest request head the proxy reads before it gives up on a client.
const MAX_HEAD: usize = 16 * 1024;
/// How long a client has to send its request head, and a host to answer the connect.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Who a token speaks for, and where it may go.
#[derive(Debug, Clone)]
struct Allowed {
    owner: String,
    hosts: Vec<String>,
}

/// The proxy, listening on loopback until dropped.
#[derive(Debug)]
pub struct NetProxy {
    port: u16,
    grants: Mutex<HashMap<String, Allowed>>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for NetProxy {
    fn drop(&mut self) {
        if let Some(task) = lock(&self.accept).take() {
            task.abort();
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One program's way through the proxy; revoked when dropped.
#[derive(Debug)]
pub struct ProxyGrant {
    token: String,
    owner: String,
    proxy: Weak<NetProxy>,
    port: u16,
}

impl Drop for ProxyGrant {
    fn drop(&mut self) {
        if let Some(proxy) = self.proxy.upgrade() {
            lock(&proxy.grants).remove(&self.token);
        }
    }
}

impl ProxyGrant {
    /// The proxy's address with this grant's credentials, as the variables carry it.
    pub fn url(&self) -> String {
        // The owner is a package id: letters, digits, `.`, `-` and `_`, safe in a URL's user part.
        format!(
            "http://{}:{}@127.0.0.1:{}",
            self.owner, self.token, self.port
        )
    }

    /// The variables a program starts with to send its requests through the proxy.
    pub fn env(&self) -> Vec<(String, String)> {
        let url = self.url();
        let mut env: Vec<(String, String)> =
            ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"]
                .iter()
                .map(|k| ((*k).to_owned(), url.clone()))
                .collect();
        env.push(("NO_PROXY".into(), String::new()));
        env.push(("no_proxy".into(), String::new()));
        // Node's own fetch reads the proxy variables only when told to.
        env.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
        env
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl NetProxy {
    /// Binds a port on loopback and serves it on the current runtime.
    pub async fn start() -> std::io::Result<Arc<NetProxy>> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let proxy = Arc::new(NetProxy {
            port,
            grants: Mutex::default(),
            accept: Mutex::default(),
        });
        let weak = Arc::downgrade(&proxy);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let Some(proxy) = weak.upgrade() else { return };
                tokio::spawn(async move { proxy.serve(stream).await });
            }
        });
        *lock(&proxy.accept) = Some(task);
        info!("[extensions] the sandbox's network proxy listens on 127.0.0.1:{port}");
        Ok(proxy)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// A way through for `owner` to `hosts` (`api.example.com`, `*.example.com`).
    pub fn grant(self: &Arc<Self>, owner: &str, hosts: Vec<String>) -> Result<ProxyGrant, String> {
        let token = crate::token::mint().map_err(|e| format!("{owner} has no proxy token: {e}"))?;
        lock(&self.grants).insert(
            token.clone(),
            Allowed {
                owner: owner.to_owned(),
                hosts,
            },
        );
        Ok(ProxyGrant {
            token,
            owner: owner.to_owned(),
            proxy: Arc::downgrade(self),
            port: self.port,
        })
    }

    fn allowed(&self, token: &str) -> Option<Allowed> {
        let grants = lock(&self.grants);
        // Every token is compared, so how long it takes says nothing about which matched.
        let mut found = None;
        for (held, allowed) in grants.iter() {
            if crate::token::same(held, token) {
                found = Some(allowed.clone());
            }
        }
        found
    }

    async fn serve(&self, mut client: TcpStream) {
        let head = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client)).await {
            Ok(Ok(head)) => head,
            _ => return,
        };
        let request = match Request::parse(&head.bytes[..head.end]) {
            Ok(r) => r,
            Err(why) => return refuse(&mut client, 400, &why).await,
        };
        let Some(allowed) = request.token.as_deref().and_then(|t| self.allowed(t)) else {
            let _ = client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"vorn\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            return;
        };
        if !declared_host(&allowed.hosts, &request.host) {
            let why = format!(
                "{} may reach only the hosts it declares, and {} is not one",
                allowed.owner, request.host
            );
            info!("[extensions] {why}");
            return refuse(&mut client, 403, &why).await;
        }
        let upstream = tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((request.host.as_str(), request.port)),
        )
        .await;
        let mut upstream = match upstream {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return refuse(&mut client, 502, &format!("{}: {e}", request.host)).await,
            Err(_) => {
                return refuse(
                    &mut client,
                    504,
                    &format!("{} did not answer", request.host),
                )
                .await
            }
        };
        let sent = if request.connect {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
        } else {
            upstream.write_all(&request.forwarded).await
        };
        if sent.is_err() {
            return;
        }
        // What the client sent past its head belongs to the host.
        if head.bytes.len() > head.end && upstream.write_all(&head.bytes[head.end..]).await.is_err()
        {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    }
}

async fn refuse(client: &mut TcpStream, status: u16, why: &str) {
    let reason = match status {
        400 => "Bad Request",
        403 => "Forbidden",
        502 => "Bad Gateway",
        _ => "Gateway Timeout",
    };
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}",
        why.len()
    );
    let _ = client.write_all(answer.as_bytes()).await;
}

/// The bytes read so far; the head ends at `end`.
struct Head {
    bytes: Vec<u8>,
    end: usize,
}

async fn read_head(client: &mut TcpStream) -> std::io::Result<Head> {
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            return Ok(Head { bytes, end: at + 4 });
        }
        if bytes.len() > MAX_HEAD {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
}

/// A request to the proxy, read from its head.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    connect: bool,
    host: String,
    port: u16,
    /// The password of `Proxy-Authorization: Basic`, which is the grant's token.
    token: Option<String>,
    /// For a plain request: the head to send the host, in origin form, the proxy's headers gone.
    forwarded: Vec<u8>,
}

impl Request {
    fn parse(head: &[u8]) -> Result<Request, String> {
        let text = std::str::from_utf8(head).map_err(|_| "a request head is text".to_owned())?;
        let mut lines = text.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let mut parts = first.split(' ');
        let (method, target, version) = match (parts.next(), parts.next(), parts.next()) {
            (Some(m), Some(t), Some(v)) if v.starts_with("HTTP/1.") => (m, t, v),
            _ => return Err(format!("{first} is not a request line")),
        };
        let mut token = None;
        let mut kept = Vec::new();
        for line in lines.filter(|l| !l.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                return Err(format!("{line} is not a header"));
            };
            if name.eq_ignore_ascii_case("proxy-authorization") {
                token = basic_password(value.trim());
            } else if !name.eq_ignore_ascii_case("proxy-connection") {
                kept.push(line);
            }
        }
        let connect = method.eq_ignore_ascii_case("CONNECT");
        let (host, port, path) = if connect {
            let (host, port) = split_port(target, None)?;
            (host, port, String::new())
        } else {
            let rest = target.strip_prefix("http://").ok_or_else(|| {
                format!("{target} is not an http:// address; https goes through CONNECT")
            })?;
            let (authority, path) = match rest.find(['/', '?', '#']) {
                Some(i) => (&rest[..i], rest[i..].to_owned()),
                None => (rest, "/".to_owned()),
            };
            let (host, port) = split_port(authority, Some(80))?;
            (host, port, path)
        };
        let mut forwarded = Vec::new();
        if !connect {
            forwarded.extend_from_slice(format!("{method} {path} {version}\r\n").as_bytes());
            for line in kept {
                forwarded.extend_from_slice(line.as_bytes());
                forwarded.extend_from_slice(b"\r\n");
            }
            forwarded.extend_from_slice(b"\r\n");
        }
        Ok(Request {
            connect,
            host,
            port,
            token,
            forwarded,
        })
    }
}

/// `host:port`, the port required unless there is a default; the host lowercase.
fn split_port(authority: &str, default: Option<u16>) -> Result<(String, u16), String> {
    if authority.contains('@') || authority.is_empty() {
        return Err(format!("{authority} is not a host to reach"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => {
            let port = p.parse().map_err(|_| format!("{p} is not a port"))?;
            (h, port)
        }
        _ => (
            authority,
            default.ok_or_else(|| format!("{authority} names no port"))?,
        ),
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Ok((host.to_ascii_lowercase(), port))
}

/// The password of a `Basic` credential.
fn basic_password(value: &str) -> Option<String> {
    let (scheme, encoded) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = data_encoding::BASE64
        .decode(encoded.trim().as_bytes())
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    text.split_once(':').map(|(_, p)| p.to_owned())
}

/// The hosts of the addresses in a setting's value: one per line or comma
/// separated, `https://` assumed without a scheme, as a feed list is written.
pub fn hosts_in(value: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for part in value
        .split(['\n', '\r', ',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let rest = match part.split_once("://") {
            Some((scheme, rest))
                if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
            {
                rest
            }
            Some(_) => continue,
            None => part,
        };
        let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = authority
            .rsplit_once(':')
            .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
            .map_or(authority, |(h, _)| h)
            .to_ascii_lowercase();
        if !host.is_empty() && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(user: &str, pass: &str) -> String {
        data_encoding::BASE64.encode(format!("{user}:{pass}").as_bytes())
    }

    #[test]
    fn reads_a_connect_and_its_credential() {
        let head = format!(
            "CONNECT Example.com:443 HTTP/1.1\r\nhost: example.com\r\nproxy-authorization: Basic {}\r\n\r\n",
            basic("ext.id", "tok")
        );
        let r = Request::parse(head.as_bytes()).unwrap();
        assert!(r.connect);
        assert_eq!((r.host.as_str(), r.port), ("example.com", 443));
        assert_eq!(r.token.as_deref(), Some("tok"));
        assert!(r.forwarded.is_empty());
    }

    #[test]
    fn forwards_a_plain_request_in_origin_form_without_the_proxy_headers() {
        let head = format!(
            "GET http://feeds.example.com:8080/rss?x=1 HTTP/1.1\r\nHost: feeds.example.com\r\nProxy-Authorization: Basic {}\r\nProxy-Connection: keep-alive\r\nAccept: */*\r\n\r\n",
            basic("a", "t")
        );
        let r = Request::parse(head.as_bytes()).unwrap();
        assert_eq!((r.host.as_str(), r.port), ("feeds.example.com", 8080));
        assert_eq!(
            String::from_utf8(r.forwarded).unwrap(),
            "GET /rss?x=1 HTTP/1.1\r\nHost: feeds.example.com\r\nAccept: */*\r\n\r\n"
        );
        let bare = Request::parse(b"GET http://a.example HTTP/1.1\r\n\r\n").unwrap();
        assert_eq!((bare.port, bare.token), (80, None));
    }

    #[test]
    fn refuses_what_is_not_a_proxy_request() {
        for head in [
            &b"GET /x HTTP/1.1\r\n\r\n"[..],
            b"CONNECT example.com HTTP/1.1\r\n\r\n",
            b"CONNECT u@example.com:443 HTTP/1.1\r\n\r\n",
            b"hello\r\n\r\n",
        ] {
            assert!(
                Request::parse(head).is_err(),
                "{}",
                String::from_utf8_lossy(head)
            );
        }
    }

    #[test]
    fn finds_the_hosts_a_feed_list_names() {
        assert_eq!(
            hosts_in("https://Blog.example.com/feed.xml\nnews.example.org/rss, http://u@x.example:8080/a ftp://no.example"),
            ["blog.example.com", "news.example.org", "x.example"]
        );
        assert!(hosts_in("").is_empty());
    }

    async fn ask(port: u16, head: String) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(head.as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out).await;
        out
    }

    #[tokio::test]
    async fn lets_a_grant_through_to_its_hosts_only_and_forgets_it_when_dropped() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let up_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = upstream.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let got = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let body = format!("saw {}", got.lines().next().unwrap_or(""));
                    let _ = s
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                });
            }
        });
        let proxy = NetProxy::start().await.unwrap();
        let grant = proxy.grant("test.feeds", vec!["localhost".into()]).unwrap();
        assert!(grant.url().starts_with("http://test.feeds:"));
        let auth = format!(
            "Proxy-Authorization: Basic {}\r\n",
            basic("test.feeds", &grant.token)
        );
        let ok = ask(
            proxy.port(),
            format!("GET http://localhost:{up_port}/x HTTP/1.1\r\nHost: localhost\r\n{auth}\r\n"),
        )
        .await;
        assert!(ok.ends_with("saw GET /x HTTP/1.1"), "{ok}");
        let tunnel = ask(
            proxy.port(),
            format!("CONNECT localhost:{up_port} HTTP/1.1\r\n{auth}\r\nPING / HTTP/1.1\r\n\r\n"),
        )
        .await;
        assert!(
            tunnel.starts_with("HTTP/1.1 200 Connection Established"),
            "{tunnel}"
        );
        assert!(tunnel.contains("saw PING / HTTP/1.1"), "{tunnel}");
        let other = ask(
            proxy.port(),
            format!("CONNECT 127.0.0.1:{up_port} HTTP/1.1\r\n{auth}\r\n"),
        )
        .await;
        assert!(other.starts_with("HTTP/1.1 403"), "{other}");
        assert!(other.ends_with(
            "test.feeds may reach only the hosts it declares, and 127.0.0.1 is not one"
        ));
        let stranger = ask(
            proxy.port(),
            format!(
                "CONNECT localhost:{up_port} HTTP/1.1\r\nProxy-Authorization: Basic {}\r\n\r\n",
                basic("x", "guess")
            ),
        )
        .await;
        assert!(stranger.starts_with("HTTP/1.1 407"), "{stranger}");
        drop(grant);
        let revoked = ask(
            proxy.port(),
            format!("CONNECT localhost:{up_port} HTTP/1.1\r\n{auth}\r\n"),
        )
        .await;
        assert!(revoked.starts_with("HTTP/1.1 407"), "{revoked}");
    }
}
