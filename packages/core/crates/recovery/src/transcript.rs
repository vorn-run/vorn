//! Recorded transcripts: real programs' output as record logs.
//!
//! A `.rec` file is what a recorder read from a PTY, one record per read, in
//! a small framed format (all integers little-endian):
//!
//! ```text
//! file    = "VREC1\n" cols:u16 rows:u16 record*
//! record  = "D" delta_us:u32 len:u32 bytes[len]     one read of output
//!         | "R" delta_us:u32 cols:u16 rows:u16      the PTY was resized
//! ```
//!
//! `delta_us` is the time since the previous record, kept for pacing replay.
//! `scripts/record.py` writes them; [`parse`] turns one into a [`Log`].

use vorn_term_proto::Record;

use crate::log::{Log, LogBuilder, Size};
use crate::Error;

const MAGIC: &[u8] = b"VREC1\n";

/// The transcripts that ship with the crate, by name.
pub const BUILTIN: &[(&str, &[u8])] = &[
    ("vim", include_bytes!("../transcripts/vim.rec")),
    ("htop", include_bytes!("../transcripts/htop.rec")),
    ("claude", include_bytes!("../transcripts/claude.rec")),
];

/// A shipped transcript by name.
pub fn builtin(name: &str) -> Result<Log, Error> {
    let (_, bytes) = BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or_else(|| Error::Log(format!("no transcript named {name}")))?;
    parse(bytes)
}

/// Every shipped transcript, with its name.
pub fn all() -> Result<Vec<(&'static str, Log)>, Error> {
    BUILTIN.iter().map(|(n, b)| Ok((*n, parse(b)?))).collect()
}

/// Reads a `.rec` file's bytes.
pub fn parse(bytes: &[u8]) -> Result<Log, Error> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err(Error::Log("not a VREC1 transcript".into()));
    }
    let size = Size::new(r.u16()?, r.u16()?);
    let mut b = LogBuilder::new(size);
    while r.at < bytes.len() {
        let tag = r.take(1)?[0];
        let delta_us = r.u32()?;
        b.advance_ns(u64::from(delta_us) * 1_000);
        match tag {
            b'D' => {
                let len = r.u32()? as usize;
                b.data(r.take(len)?);
            }
            b'R' => {
                let size = Size::new(r.u16()?, r.u16()?);
                b.resize(size);
            }
            other => {
                return Err(Error::Log(format!(
                    "unknown record tag {other:#x} at byte {}",
                    r.at - 5
                )))
            }
        }
    }
    let log = b.build();
    log.validate()?;
    Ok(log)
}

/// Writes a log as a `.rec` file. Only Data and Resize records have a form;
/// anything else is refused.
pub fn encode(log: &Log) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(log.data_len() as usize + 16 * log.entries.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&log.size.cols.to_le_bytes());
    out.extend_from_slice(&log.size.rows.to_le_bytes());
    let mut last_ns = 0u64;
    for e in &log.entries {
        let delta = u32::try_from(e.at_ns.saturating_sub(last_ns) / 1_000).unwrap_or(u32::MAX);
        last_ns = e.at_ns;
        match &e.rec {
            Record::Data { bytes, .. } => {
                out.push(b'D');
                out.extend_from_slice(&delta.to_le_bytes());
                let len = u32::try_from(bytes.len())
                    .map_err(|_| Error::Log("record over 4 GiB".into()))?;
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(bytes);
            }
            &Record::Resize { cols, rows, .. } => {
                out.push(b'R');
                out.extend_from_slice(&delta.to_le_bytes());
                out.extend_from_slice(&cols.to_le_bytes());
                out.extend_from_slice(&rows.to_le_bytes());
            }
            Record::Gap { .. } | Record::Exit { .. } => {
                return Err(Error::Unsupported("Gap and Exit records in a transcript"))
            }
        }
    }
    Ok(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or_else(|| {
            Error::Log(format!(
                "transcript ends inside a record at byte {}",
                self.at
            ))
        })?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_log() {
        let mut b = LogBuilder::new(Size::new(80, 24));
        b.data("hi\x1b[")
            .advance_ns(5_000)
            .resize(Size::new(90, 30))
            .data("31m");
        let log = b.build();
        let back = parse(&encode(&log).unwrap()).unwrap();
        assert_eq!(back, log);
    }

    #[test]
    fn refuses_a_truncated_file() {
        let mut b = LogBuilder::new(Size::new(80, 24));
        b.data("hello");
        let bytes = encode(&b.build()).unwrap();
        assert!(parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(parse(b"VREC2\n").is_err());
    }

    #[test]
    fn the_shipped_transcripts_parse() {
        for (name, log) in all().unwrap() {
            assert!(log.entries.len() > 10, "{name}");
        }
    }
}
