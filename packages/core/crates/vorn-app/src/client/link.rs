//! How frames reach vornd: the RPC and the grid speak through a [`Transport`],
//! a local socket or WebSocket on a desktop and the page's WebSockets in a
//! browser, so neither knows which it has.

use tokio::sync::mpsc;

/// One message either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    /// Text the link writes out, so a large message is not serialized on the UI thread.
    Json(serde_json::Value),
}

/// Why a link ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// vornd refused the credential.
    Rejected,
    /// Anything else: vornd went away, the network dropped, a frame was bad.
    Closed,
}

/// The close code vornd ends a socket with when it refuses its credential.
pub const CLOSE_CREDENTIAL_REJECTED: u16 = 4002;

/// What a link hands back: each frame vornd sends (false hangs up), then
/// once, why it ended.
pub struct Inbound {
    pub frame: Box<dyn FnMut(Frame) -> bool + Send>,
    pub ended: Box<dyn FnOnce(Ended) + Send>,
}

/// A way to vornd. `start` returns once the link is under way; the frames
/// on `out` go to vornd in order, and what comes back goes to `inbound`.
pub trait Transport {
    fn start(
        self: Box<Self>,
        out: mpsc::UnboundedReceiver<Frame>,
        inbound: Inbound,
    ) -> Result<(), String>;
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{LocalSocket, Socket};

#[cfg(target_arch = "wasm32")]
pub use page::PageSocket;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::WebSocketStream;
    use vorn_reach::tls::rustls::{self, pki_types::ServerName};

    use super::{Ended, Frame, Inbound, Transport, CLOSE_CREDENTIAL_REJECTED};
    use crate::client::rpc::CALL_TIMEOUT;

    fn runtime() -> Result<tokio::runtime::Runtime, String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| e.to_string())
    }

    /// vornd's `/ws`, the token on the upgrade; over TLS trusting only the
    /// certificate `pin` names, when there is one.
    pub struct Socket {
        pub url: String,
        pub token: String,
        pub pin: Option<String>,
    }

    impl Transport for Socket {
        fn start(
            self: Box<Self>,
            out: mpsc::UnboundedReceiver<Frame>,
            inbound: Inbound,
        ) -> Result<(), String> {
            let rt = runtime()?;
            let mut req = self.url.into_client_request().map_err(|e| e.to_string())?;
            let auth = format!("Bearer {}", self.token)
                .parse()
                .map_err(|_| "the token is not a header value".to_owned())?;
            req.headers_mut().insert("authorization", auth);
            match self.pin {
                None => {
                    let ws = rt
                        .block_on(async {
                            tokio::time::timeout(
                                CALL_TIMEOUT,
                                tokio_tungstenite::connect_async(req),
                            )
                            .await
                        })
                        .map_err(|_| "vornd did not answer in time".to_owned())?
                        .map_err(|e| e.to_string())?
                        .0;
                    relay(rt, ws, out, inbound)
                }
                Some(pin) => {
                    let ws = rt
                        .block_on(async {
                            tokio::time::timeout(CALL_TIMEOUT, connect_pinned(req, &pin)).await
                        })
                        .map_err(|_| "vornd did not answer in time".to_owned())??;
                    relay(rt, ws, out, inbound)
                }
            }
        }
    }

    /// Opens `req` over TLS, trusting only the certificate `pin` names.
    async fn connect_pinned(
        req: tokio_tungstenite::tungstenite::handshake::client::Request,
        pin: &str,
    ) -> Result<WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>, String> {
        let config = vorn_reach::tls::client_config(pin).map_err(|e| e.to_string())?;
        let host = req
            .uri()
            .host()
            .ok_or("the address has no host")?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned();
        let port = req.uri().port_u16().unwrap_or(443);
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|e| e.to_string())?;
        let name = ServerName::try_from(host).map_err(|e| e.to_string())?;
        let tls = tokio_rustls::TlsConnector::from(config)
            .connect(name, tcp)
            .await
            .map_err(
                |e| match e.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()) {
                    Some(rustls::Error::InvalidCertificate(_)) => {
                        "This computer's certificate changed. Pair again.".to_owned()
                    }
                    _ => e.to_string(),
                },
            )?;
        let (ws, _) = tokio_tungstenite::client_async(req, tls)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ws)
    }

    /// Carries frames both ways on its own thread until either side ends.
    fn relay<S>(
        rt: tokio::runtime::Runtime,
        mut ws: WebSocketStream<S>,
        mut out: mpsc::UnboundedReceiver<Frame>,
        mut inbound: Inbound,
    ) -> Result<(), String>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        std::thread::Builder::new()
            .name("vornd-rpc".into())
            .spawn(move || {
                rt.block_on(async move {
                    let mut ended = Ended::Closed;
                    loop {
                        tokio::select! {
                            msg = ws.next() => {
                                let frame = match msg {
                                    Some(Ok(Message::Text(t))) => Frame::Text(t.as_str().to_owned()),
                                    Some(Ok(Message::Binary(b))) => Frame::Binary(b.to_vec()),
                                    Some(Ok(Message::Close(c))) => {
                                        if c.is_some_and(|c| c.code == CloseCode::from(CLOSE_CREDENTIAL_REJECTED)) {
                                            ended = Ended::Rejected;
                                        }
                                        break;
                                    }
                                    Some(Err(_)) | None => break,
                                    Some(Ok(_)) => continue,
                                };
                                if !(inbound.frame)(frame) {
                                    break;
                                }
                            }
                            frame = out.recv() => {
                                let msg = match frame {
                                    Some(Frame::Text(t)) => Message::text(t),
                                    Some(Frame::Json(v)) => Message::text(v.to_string()),
                                    Some(Frame::Binary(b)) => Message::binary(b),
                                    None => {
                                        let _ = ws.close(None).await;
                                        break;
                                    }
                                };
                                if ws.send(msg).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    (inbound.ended)(ended);
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// vornd's grid endpoint: a Unix socket, or a named pipe on Windows.
    /// Text frames have no meaning on it and are dropped.
    pub struct LocalSocket(pub String);

    impl Transport for LocalSocket {
        fn start(
            self: Box<Self>,
            out: mpsc::UnboundedReceiver<Frame>,
            inbound: Inbound,
        ) -> Result<(), String> {
            let rt = runtime()?;
            let conn = rt.block_on(open(&self.0)).map_err(|e| e.to_string())?;
            std::thread::Builder::new()
                .name("grid-io".into())
                .spawn(move || rt.block_on(pump(conn, out, inbound)))
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    /// Reads and writes on one task: a synchronous Windows pipe handle
    /// would serialize a blocking read against every write.
    async fn pump(conn: Conn, mut out: mpsc::UnboundedReceiver<Frame>, mut inbound: Inbound) {
        let (mut rd, mut wr) = tokio::io::split(conn);
        let mut buf = vec![0u8; 256 << 10];
        loop {
            tokio::select! {
                r = rd.read(&mut buf) => match r {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if !(inbound.frame)(Frame::Binary(buf[..n].to_vec())) {
                            break;
                        }
                    }
                },
                frame = out.recv() => match frame {
                    Some(Frame::Binary(bytes)) => {
                        if wr.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Frame::Text(_) | Frame::Json(_)) => {}
                    None => break,
                }
            }
        }
        (inbound.ended)(Ended::Closed);
    }

    trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
    impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}
    type Conn = Box<dyn Stream>;

    #[cfg(unix)]
    async fn open(endpoint: &str) -> std::io::Result<Conn> {
        Ok(Box::new(tokio::net::UnixStream::connect(endpoint).await?))
    }

    #[cfg(windows)]
    async fn open(endpoint: &str) -> std::io::Result<Conn> {
        use std::time::{Duration, Instant};
        use tokio::net::windows::named_pipe::ClientOptions;
        const ERROR_PIPE_BUSY: i32 = 231;
        let started = Instant::now();
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(c) => return Ok(Box::new(c)),
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && started.elapsed() < Duration::from_secs(5) =>
                {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod page {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;
    use tokio::sync::{mpsc, oneshot};
    use wasm_bindgen::prelude::*;
    use web_sys::{BinaryType, CloseEvent, MessageEvent, WebSocket};

    use super::{Ended, Frame, Inbound, Transport, CLOSE_CREDENTIAL_REJECTED};

    /// One of the page's WebSockets to vornd (`/ws` or `/grid`). A browser
    /// cannot set a header on the upgrade, so the token goes first, as an
    /// `auth:authenticate` frame, and the rest queue behind it.
    pub struct PageSocket {
        pub url: String,
        pub token: String,
    }

    impl Transport for PageSocket {
        fn start(
            self: Box<Self>,
            mut out: mpsc::UnboundedReceiver<Frame>,
            inbound: Inbound,
        ) -> Result<(), String> {
            let ws = WebSocket::new(&self.url).map_err(|e| format!("{e:?}"))?;
            ws.set_binary_type(BinaryType::Arraybuffer);
            let Inbound { mut frame, ended } = inbound;
            let ended = Rc::new(RefCell::new(Some(ended)));
            let end = {
                let ended = Rc::clone(&ended);
                move |why: Ended| {
                    if let Some(f) = ended.borrow_mut().take() {
                        f(why);
                    }
                }
            };
            let (opened_tx, opened) = oneshot::channel::<bool>();
            let opened_tx = Rc::new(RefCell::new(Some(opened_tx)));

            let on_open = {
                let tx = Rc::clone(&opened_tx);
                Closure::<dyn FnMut()>::new(move || {
                    if let Some(tx) = tx.borrow_mut().take() {
                        let _ = tx.send(true);
                    }
                })
            };
            let on_message = {
                let ws = ws.clone();
                Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                    let data = e.data();
                    let got = if let Some(text) = data.as_string() {
                        Frame::Text(text)
                    } else if let Ok(buf) = data.dyn_into::<js_sys::ArrayBuffer>() {
                        Frame::Binary(js_sys::Uint8Array::new(&buf).to_vec())
                    } else {
                        return;
                    };
                    if !frame(got) {
                        let _ = ws.close();
                    }
                })
            };
            let on_close = {
                let end = end.clone();
                let tx = Rc::clone(&opened_tx);
                Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
                    if let Some(tx) = tx.borrow_mut().take() {
                        let _ = tx.send(false);
                    }
                    end(if e.code() == CLOSE_CREDENTIAL_REJECTED {
                        Ended::Rejected
                    } else {
                        Ended::Closed
                    });
                })
            };
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

            let auth = json!({
                "jsonrpc": "2.0",
                "method": "auth:authenticate",
                "params": { "token": self.token },
            })
            .to_string();
            wasm_bindgen_futures::spawn_local(async move {
                // The closures live as long as the socket's pump.
                let _held = (on_open, on_message, on_close);
                let open = opened.await.unwrap_or(false) && ws.send_with_str(&auth).is_ok();
                while let Some(f) = if open { out.recv().await } else { None } {
                    let sent = match f {
                        Frame::Text(t) => ws.send_with_str(&t),
                        Frame::Json(v) => ws.send_with_str(&v.to_string()),
                        Frame::Binary(b) => ws.send_with_u8_array(&b),
                    };
                    if sent.is_err() {
                        break;
                    }
                }
                ws.set_onopen(None);
                ws.set_onmessage(None);
                ws.set_onclose(None);
                let _ = ws.close();
                end(Ended::Closed);
            });
            Ok(())
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    use futures_util::SinkExt;
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::Message;
    use vorn_reach::tls::{server_config, Identity};

    use super::{Ended, Frame, Inbound, Socket, Transport};

    /// A TLS WebSocket server that greets once; its port and fingerprint.
    fn greeter() -> (u16, String) {
        let id = Identity::generate().unwrap();
        let fingerprint = id.fingerprint();
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config(&id).unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                while let Ok((tcp, _)) = listener.accept().await {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        continue;
                    };
                    let mut ws = tokio_tungstenite::accept_async(tls).await.unwrap();
                    ws.send(Message::text("hello")).await.unwrap();
                }
            });
        });
        (port, fingerprint)
    }

    /// The frames that arrive, and the way out, kept open while held.
    type Opened = (std_mpsc::Receiver<Frame>, mpsc::UnboundedSender<Frame>);

    fn start(port: u16, pin: &str) -> Result<Opened, String> {
        let (tx, rx) = std_mpsc::channel();
        let (out, out_rx) = mpsc::unbounded_channel();
        let socket = Socket {
            url: format!("wss://127.0.0.1:{port}/ws"),
            token: "t".into(),
            pin: Some(pin.into()),
        };
        let inbound = Inbound {
            frame: Box::new(move |f| tx.send(f).is_ok()),
            ended: Box::new(|_: Ended| {}),
        };
        Box::new(socket).start(out_rx, inbound)?;
        Ok((rx, out))
    }

    #[test]
    fn a_socket_connects_only_to_the_certificate_it_pinned() {
        let (port, fingerprint) = greeter();
        let (frames, _out) = start(port, &fingerprint).expect("the pinned certificate is trusted");
        assert_eq!(
            frames.recv_timeout(Duration::from_secs(5)).unwrap(),
            Frame::Text("hello".into())
        );
        let other = Identity::generate().unwrap().fingerprint();
        let refused = start(port, &other).expect_err("any other is refused");
        assert_eq!(refused, "This computer's certificate changed. Pair again.");
    }
}
