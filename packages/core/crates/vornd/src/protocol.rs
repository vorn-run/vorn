//! The version vornd speaks, which a client reads from the `Vornd-Protocol` upgrade header.

/// vornd's own version, sent as the `Vornd-Protocol` header when a WebSocket
/// is accepted and reported by the health check. It goes up when what vornd adds
/// to the wire changes, not with the app version.
pub const VORND_PROTOCOL: u64 = 1;
