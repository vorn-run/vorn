//! History log frames, byte for byte as `packages/server/src/history/log.ts`
//! writes them: a kind, the payload's length and its CRC-32, then the payload,
//! which starts with the record's number and the byte offset it starts at.
//!
//! The reader stays in TypeScript, so this side only ever has to agree with it;
//! `tests/history-pipeline.test.ts` reads frames built here.

/// Kind byte, payload length, CRC-32.
pub const PREFIX_BYTES: usize = 1 + 4 + 4;
/// `rseq` and `startOffset`, both u64.
pub const RECORD_HEADER_BYTES: usize = 16;

const KIND_DATA: u8 = 0x10;
const KIND_RESIZE: u8 = 0x11;
/// Output from the PTY, as opposed to anything Vorn writes into the stream.
pub const STREAM_PTY: u8 = 0;

/// Where a record sits in the session's stream: the Session Recovery
/// Contract's `RecordHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub rseq: u64,
    pub start_offset: u64,
}

/// Append one output record to `out`.
pub fn data(out: &mut Vec<u8>, at: Record, bytes: &[u8]) {
    let len = RECORD_HEADER_BYTES + 1 + bytes.len();
    let start = begin(out, KIND_DATA, len, at);
    out.push(STREAM_PTY);
    out.extend_from_slice(bytes);
    finish(out, start);
}

/// Append one resize record to `out`. Pixel sizes are written as zero, as the
/// TypeScript writer does.
pub fn resize(out: &mut Vec<u8>, at: Record, cols: u16, rows: u16) {
    let start = begin(out, KIND_RESIZE, RECORD_HEADER_BYTES + 8, at);
    out.extend_from_slice(&cols.to_le_bytes());
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    finish(out, start);
}

/// The prefix with a placeholder checksum, and the record header. Returns
/// where the frame starts.
fn begin(out: &mut Vec<u8>, kind: u8, len: usize, at: Record) -> usize {
    let start = out.len();
    out.reserve(PREFIX_BYTES + len);
    out.push(kind);
    // A record is one flush, at most 64 KB of output; u32 is never short.
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&at.rseq.to_le_bytes());
    out.extend_from_slice(&at.start_offset.to_le_bytes());
    start
}

/// Checksum the payload in place, now that it is all there.
fn finish(out: &mut [u8], start: usize) {
    let crc = crc32fast::hash(&out[start + PREFIX_BYTES..]);
    out[start + 5..start + PREFIX_BYTES].copy_from_slice(&crc.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_ieee_crc32() {
        // The check value every IEEE CRC-32 is tested against, as log.ts pins it.
        assert_eq!(crc32fast::hash(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn lays_out_a_data_record() {
        let mut out = Vec::new();
        data(
            &mut out,
            Record {
                rseq: 7,
                start_offset: 100,
            },
            b"hi",
        );
        let payload_len = RECORD_HEADER_BYTES + 1 + 2;
        assert_eq!(out.len(), PREFIX_BYTES + payload_len);
        assert_eq!(out[0], KIND_DATA);
        assert_eq!(
            u32::from_le_bytes(out[1..5].try_into().unwrap()) as usize,
            payload_len
        );
        let payload = &out[PREFIX_BYTES..];
        assert_eq!(
            u32::from_le_bytes(out[5..9].try_into().unwrap()),
            crc32fast::hash(payload)
        );
        assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(payload[8..16].try_into().unwrap()), 100);
        assert_eq!(payload[16], STREAM_PTY);
        assert_eq!(&payload[17..], b"hi");
    }

    #[test]
    fn lays_out_a_resize_record() {
        let mut out = vec![0xaa];
        resize(
            &mut out,
            Record {
                rseq: 1,
                start_offset: 2,
            },
            200,
            50,
        );
        let frame = &out[1..];
        assert_eq!(frame[0], KIND_RESIZE);
        let payload = &frame[PREFIX_BYTES..];
        assert_eq!(payload.len(), RECORD_HEADER_BYTES + 8);
        assert_eq!(&payload[16..20], &[200, 0, 50, 0]);
        assert_eq!(&payload[20..], &[0; 4]);
        assert_eq!(
            u32::from_le_bytes(frame[5..9].try_into().unwrap()),
            crc32fast::hash(payload)
        );
    }
}
