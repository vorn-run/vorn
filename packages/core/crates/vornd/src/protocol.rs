//! What vornd reads in the frames it forwards, and the versions it speaks.
//!
//! vornd forwards frames as they are, and changes none of them. It reads two
//! things from them: the method of each call a client makes, to count it against
//! its group, and the server's `server:hello`, to check the server speaks a
//! protocol version vornd knows. A client that wants to know it is behind vornd
//! reads the `Vornd-Protocol` header on the upgrade response, so nothing in the
//! frames a client already parses differs from talking to the server itself.

use serde_json::Value;

/// vornd's own version, sent as the `Vornd-Protocol` header when a WebSocket
/// is accepted and reported by the health check. It goes up when what vornd adds
/// to the wire changes, not with the app version.
pub const VORND_PROTOCOL: u64 = 1;

/// The server `protocolVersion`s vornd can stand in front of.
pub const SERVER_PROTOCOLS: std::ops::RangeInclusive<u64> = 1..=1;

/// The method a client frame calls, for a request or a notification.
pub fn method_of(text: &str) -> Option<String> {
    // Cheap test first: most frames a server sends are not calls, and the ones
    // a client sends are small.
    if !text.contains("\"method\"") {
        return None;
    }
    match serde_json::from_str::<Value>(text).ok()? {
        Value::Object(map) => map.get("method")?.as_str().map(str::to_owned),
        _ => None,
    }
}

/// What to do with a frame from the server.
#[derive(Debug, PartialEq, Eq)]
pub enum ServerFrame {
    /// Forward it unchanged.
    Pass,
    /// The hello names a protocol vornd does not know.
    Unsupported(Option<u64>),
}

pub fn inspect_server_frame(text: &str) -> ServerFrame {
    if !text.contains("server:hello") {
        return ServerFrame::Pass;
    }
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
        return ServerFrame::Pass;
    };
    if frame.get("method").and_then(Value::as_str) != Some("server:hello")
        || frame.contains_key("id")
    {
        return ServerFrame::Pass;
    }
    let version = frame
        .get("params")
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_u64);
    match version {
        Some(v) if SERVER_PROTOCOLS.contains(&v) => ServerFrame::Pass,
        other => ServerFrame::Unsupported(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_method_of_requests_and_notifications() {
        assert_eq!(
            method_of(r#"{"jsonrpc":"2.0","id":3,"method":"git:status","params":{}}"#).as_deref(),
            Some("git:status")
        );
        assert_eq!(
            method_of(r#"{"jsonrpc":"2.0","method":"terminal:input","params":{"id":"a"}}"#)
                .as_deref(),
            Some("terminal:input")
        );
        assert_eq!(
            method_of(r#"{"jsonrpc":"2.0","id":3,"result":{"method":"x"}}"#),
            None
        );
        assert_eq!(method_of("not json \"method\""), None);
    }

    #[test]
    fn passes_a_hello_it_supports_unchanged() {
        let hello = r#"{"jsonrpc":"2.0","method":"server:hello","params":{"protocolVersion":1,"capabilities":{"auth":1}}}"#;
        assert_eq!(inspect_server_frame(hello), ServerFrame::Pass);
    }

    #[test]
    fn refuses_a_server_protocol_it_does_not_know() {
        let hello = r#"{"jsonrpc":"2.0","method":"server:hello","params":{"protocolVersion":2}}"#;
        assert_eq!(
            inspect_server_frame(hello),
            ServerFrame::Unsupported(Some(2))
        );
        let bare = r#"{"jsonrpc":"2.0","method":"server:hello"}"#;
        assert_eq!(inspect_server_frame(bare), ServerFrame::Unsupported(None));
    }

    #[test]
    fn passes_everything_else() {
        assert_eq!(
            inspect_server_frame(r#"{"jsonrpc":"2.0","id":1,"result":"server:hello"}"#),
            ServerFrame::Pass
        );
        assert_eq!(
            inspect_server_frame(r#"{"jsonrpc":"2.0","method":"server:identity","params":{}}"#),
            ServerFrame::Pass
        );
    }
}
