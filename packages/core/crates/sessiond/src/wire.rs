//! The socket protocol between vornd and sessiond (RC §5).
//!
//! A frame is a 4-byte little-endian length, then a 1-byte message type, then
//! a postcard-encoded body; the length counts the type byte and the body.
//! Message types never change meaning: a new field is a new type or a newer
//! `proto`. A frame this build cannot read closes the connection, never the
//! process (RC-T15).

use serde::{Deserialize, Serialize};
use vorn_term_proto::{Cursor, Entry};

/// The protocol version this sessiond speaks. sessiond implements exactly
/// one; vornd implements every version back to the oldest sessiond still
/// allowed to run.
pub const PROTO: u16 = 1;

/// The largest frame either side accepts. Entries are batched below it.
pub const MAX_FRAME: usize = 16 << 20;

pub type SessionId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto_min: u16,
    pub proto_max: u16,
    pub vornd_instance: u128,
    pub vornd_build: String,
}

/// Where an attach starts reading a session's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachFrom {
    /// The newest checkpoint and every record from its resume cursor.
    NewestCheckpoint,
    /// The older checkpoint, when the newest fails its restore check.
    FallbackCheckpoint,
    /// rseq 0, while it is still retained.
    SessionStart,
    /// Records from `next_rseq`; refused outside `[oldest, head]`.
    Cursor(Cursor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachRefusal {
    NoSuchSession,
    NoSuchCheckpoint,
    NotRetained,
    WrongEpoch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attach {
    pub session: SessionId,
    pub from: AttachFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stdin {
    Pipe,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Io {
    Pty { cols: u16, rows: u16 },
    Piped { stdin: Stdin },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSpec {
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub io: Io,
    pub ring_bytes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawn {
    pub req: u64,
    pub spec: SpawnSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub session: SessionId,
    /// Numbered per vornd attach. Input is at-most-once: nothing is retried.
    pub input_seq: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    pub session: SessionId,
    pub req: u64,
    pub cols: u16,
    pub rows: u16,
    pub px_w: u16,
    pub px_h: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sig {
    Int,
    Term,
    Kill,
    Hup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    pub session: SessionId,
    pub signal: Sig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    pub session: SessionId,
    pub delivered: Cursor,
}

/// A state vornd cut, opaque to sessiond apart from where it ends and its CRC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub session: SessionId,
    /// The first record and byte this state does not include.
    pub resume: Cursor,
    pub cols: u16,
    pub rows: u16,
    /// vornd's checkpoint format; sessiond never reads it.
    pub format: u16,
    pub vornd_build: String,
    pub blob_crc32: u32,
    pub blob: Vec<u8>,
}

impl Checkpoint {
    pub fn crc_ok(&self) -> bool {
        crc32fast::hash(&self.blob) == self.blob_crc32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nonce {
    pub nonce: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Pty,
    Piped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session: SessionId,
    pub kind: Kind,
    pub pid: u32,
    pub epoch: u32,
    pub oldest: Cursor,
    pub head: Cursor,
    pub newest_cp: Option<Cursor>,
    pub retain_from: Cursor,
    /// After the last record written to the previous vornd's socket.
    pub sent: Cursor,
    pub cols: u16,
    pub rows: u16,
    pub exited: Option<ExitInfo>,
    pub spooled_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub proto: u16,
    pub sessiond_instance: u128,
    pub sessiond_build: String,
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub session: SessionId,
    pub why: AttachRefusal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entries {
    pub session: SessionId,
    /// Contiguous by rseq.
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawned {
    pub req: u64,
    pub session: SessionId,
    pub pid: u32,
    pub start: Cursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    pub req: u64,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDone {
    pub session: SessionId,
    pub input_seq: u64,
    /// Bytes the kernel accepted.
    pub written: u32,
}

/// Why a frame was not read. Each one closes the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// A declared length over [`MAX_FRAME`], or zero.
    BadLength(u32),
    UnknownType(u8),
    BadBody(u8),
}

/// A set of messages that travel in one direction, each with its fixed type byte.
pub trait Message: Sized {
    fn kind(&self) -> u8;
    fn body(&self) -> postcard::Result<Vec<u8>>;
    fn read(kind: u8, body: &[u8]) -> Result<Self, WireError>;

    /// The whole frame: length, type, body.
    fn encode(&self) -> Vec<u8> {
        let body = self.body().expect("every message serializes");
        let mut out = Vec::with_capacity(5 + body.len());
        out.extend_from_slice(&(1 + body.len() as u32).to_le_bytes());
        out.push(self.kind());
        out.extend_from_slice(&body);
        out
    }
}

macro_rules! messages {
    ($(#[$meta:meta])* $name:ident { $($variant:ident($body:ty) = $code:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name { $($variant($body),)* }

        impl Message for $name {
            fn kind(&self) -> u8 {
                match self { $($name::$variant(_) => $code,)* }
            }
            fn body(&self) -> postcard::Result<Vec<u8>> {
                match self { $($name::$variant(b) => postcard::to_stdvec(b),)* }
            }
            fn read(kind: u8, body: &[u8]) -> Result<Self, WireError> {
                match kind {
                    $($code => postcard::from_bytes::<$body>(body)
                        .map($name::$variant)
                        .map_err(|_| WireError::BadBody(kind)),)*
                    other => Err(WireError::UnknownType(other)),
                }
            }
        }
    };
}

messages! {
    /// vornd → sessiond.
    ToSessiond {
        Hello(Hello) = 0x01,
        Attach(Attach) = 0x02,
        Spawn(Spawn) = 0x03,
        Write(Write) = 0x04,
        CloseStdin(SessionRef) = 0x05,
        Resize(Resize) = 0x06,
        Signal(Signal) = 0x07,
        Ack(Ack) = 0x08,
        PutCheckpoint(Checkpoint) = 0x09,
        Release(SessionRef) = 0x0a,
        Ping(Nonce) = 0x0b,
    }
}

messages! {
    /// sessiond → vornd.
    ToVornd {
        Welcome(Welcome) = 0x81,
        CheckpointIs(Checkpoint) = 0x82,
        Refused(Refused) = 0x83,
        Entries(Entries) = 0x84,
        Spawned(Spawned) = 0x85,
        Failed(Failed) = 0x86,
        InputDone(InputDone) = 0x87,
        Pong(Nonce) = 0x88,
    }
}

/// Splits a byte stream into messages. Feed it whatever a read returned; it
/// never allocates for a frame before its bytes have arrived.
#[derive(Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    start: usize,
}

impl FrameReader {
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start * 2 > self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, `None` while one is still arriving.
    pub fn read<M: Message>(&mut self) -> Result<Option<M>, WireError> {
        let rest = &self.buf[self.start..];
        let Some(len) = rest.get(..4) else {
            return Ok(None);
        };
        let len = u32::from_le_bytes(len.try_into().unwrap());
        if len == 0 || len as usize > MAX_FRAME {
            return Err(WireError::BadLength(len));
        }
        let Some(frame) = rest.get(4..4 + len as usize) else {
            return Ok(None);
        };
        let msg = M::read(frame[0], &frame[1..])?;
        self.start += 4 + len as usize;
        Ok(Some(msg))
    }

    /// Bytes received and not yet read as a message.
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vorn_term_proto::{Record, RecordHeader, Stream};

    fn cursor(r: u64, o: u64) -> Cursor {
        Cursor {
            epoch: 1,
            next_rseq: r,
            next_offset: o,
        }
    }

    fn samples_to_sessiond() -> Vec<ToSessiond> {
        vec![
            ToSessiond::Hello(Hello {
                proto_min: 1,
                proto_max: 3,
                vornd_instance: u128::MAX - 7,
                vornd_build: "0.8.0".into(),
            }),
            ToSessiond::Attach(Attach {
                session: "s1".into(),
                from: AttachFrom::Cursor(cursor(8, 120)),
            }),
            ToSessiond::Spawn(Spawn {
                req: 3,
                spec: SpawnSpec {
                    argv: vec!["zsh".into(), "-l".into()],
                    cwd: "/tmp".into(),
                    env: vec![("TERM".into(), "xterm-256color".into())],
                    io: Io::Piped { stdin: Stdin::Pipe },
                    ring_bytes: Some(8 << 20),
                },
            }),
            ToSessiond::Write(Write {
                session: "s1".into(),
                input_seq: 9,
                bytes: b"ls\r".to_vec(),
            }),
            ToSessiond::PutCheckpoint(Checkpoint {
                session: "s1".into(),
                resume: cursor(200, 4096),
                cols: 80,
                rows: 24,
                format: 1,
                vornd_build: "b".into(),
                blob_crc32: crc32fast::hash(b"blob"),
                blob: b"blob".to_vec(),
            }),
            ToSessiond::Ping(Nonce { nonce: 42 }),
        ]
    }

    #[test]
    fn messages_round_trip_through_split_reads() {
        let mut stream = Vec::new();
        let sent = samples_to_sessiond();
        for m in &sent {
            stream.extend(m.encode());
        }
        let reply = ToVornd::Entries(Entries {
            session: "s1".into(),
            entries: vec![Entry {
                hdr: RecordHeader {
                    epoch: 1,
                    rseq: 7,
                    start_offset: 100,
                },
                at_ns: 5,
                rec: Record::Data {
                    stream: Stream::Stderr,
                    bytes: vec![0, 255, 27],
                },
            }],
        });
        // One byte at a time: nothing is read before its frame is whole.
        let mut r = FrameReader::default();
        let mut got = Vec::new();
        for b in &stream {
            r.push(std::slice::from_ref(b));
            while let Some(m) = r.read::<ToSessiond>().unwrap() {
                got.push(m);
            }
        }
        assert_eq!(got, sent);
        assert_eq!(r.pending(), 0);
        r.push(&reply.encode());
        assert_eq!(r.read::<ToVornd>().unwrap(), Some(reply));
    }

    #[test]
    fn a_frame_from_the_other_direction_is_refused() {
        let mut r = FrameReader::default();
        r.push(&ToVornd::Pong(Nonce { nonce: 1 }).encode());
        assert_eq!(r.read::<ToSessiond>(), Err(WireError::UnknownType(0x88)));
    }

    #[test]
    fn oversized_and_empty_frames_are_errors_before_any_allocation() {
        let mut r = FrameReader::default();
        r.push(&(MAX_FRAME as u32 + 1).to_le_bytes());
        assert_eq!(
            r.read::<ToSessiond>(),
            Err(WireError::BadLength(MAX_FRAME as u32 + 1))
        );
        let mut r = FrameReader::default();
        r.push(&[0, 0, 0, 0]);
        assert_eq!(r.read::<ToSessiond>(), Err(WireError::BadLength(0)));
    }

    /// RC-T15 at unit level: malformed input is an error, never a panic.
    /// Random bytes, random type bytes on valid lengths, and every valid
    /// frame truncated or with one byte flipped.
    #[test]
    fn malformed_frames_never_panic() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..5_000 {
            let n = rng.below(64) as usize;
            let junk: Vec<u8> = (0..n).map(|_| rng.below(256) as u8).collect();
            let mut r = FrameReader::default();
            r.push(&junk);
            while let Ok(Some(_)) = r.read::<ToSessiond>() {}
            let mut framed = (1 + junk.len() as u32).to_le_bytes().to_vec();
            framed.push(rng.below(16) as u8);
            framed.extend(&junk);
            let mut r = FrameReader::default();
            r.push(&framed);
            let _ = r.read::<ToSessiond>();
        }
        for m in samples_to_sessiond() {
            let frame = m.encode();
            for i in 4..frame.len() {
                let mut bad = frame.clone();
                bad[i] ^= 1 << rng.below(8);
                let mut r = FrameReader::default();
                r.push(&bad);
                let _ = r.read::<ToSessiond>();
                let mut r = FrameReader::default();
                r.push(&frame[..i]);
                assert_eq!(r.read::<ToSessiond>(), Ok(None));
            }
        }
    }

    struct Rng(u64);
    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }
}
