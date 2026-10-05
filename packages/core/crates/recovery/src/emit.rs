//! A program that prints seeded output for a real terminal to record:
//! `recovery-emit`, this crate's second binary, runs [`run`] under sessiond
//! so an end-to-end test can kill vornd while it prints and compare what was
//! recorded with what was generated.
//!
//! What it prints is [`output`]: the Data records of
//! `Generator::log(seed, Profile::round_trip().bytes(bytes))` in order, made
//! valid UTF-8 ([`to_utf8`]). The generator ends some strings with the C1
//! byte ST (0x9c), which is not UTF-8, and Rust's standard output on a
//! Windows console (a ConPTY session is one) writes through `WriteConsoleW`
//! and refuses bytes that are not UTF-8. Writing the raw handle instead
//! would hand the bytes to the console's code page, which is not UTF-8 by
//! default, and mangle every non-ASCII character. So C1 controls are
//! written in their 7-bit form (ST as `ESC \`), which a terminal reads the
//! same, and the output is cut only on character boundaries ([`chunks`]).
//! On a pipe the bytes arrive exactly as [`output`] returns them.
//!
//! On a Windows console it also turns on VT processing, so the console host
//! interprets the escape sequences instead of printing them as text.

use std::io::{self, Read, Write};
use std::time::Duration;

use vorn_term_proto::Record;

use crate::gen::{Generator, Profile};

/// Everything `recovery-emit <seed> <bytes> ...` prints, in order.
pub fn output(seed: u64, bytes: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(usize::try_from(bytes).unwrap_or(0));
    for e in Generator::new(seed, Profile::round_trip().bytes(bytes)) {
        if let Record::Data { bytes, .. } = e.rec {
            out.extend_from_slice(&bytes);
        }
    }
    to_utf8(out)
}

/// `bytes` as valid UTF-8, unchanged when they already are. A byte that is
/// not part of a character becomes its 7-bit form when it is a C1 control
/// (0x80 to 0x9f become ESC and the byte less 0x40), and U+FFFD otherwise.
pub fn to_utf8(bytes: Vec<u8>) -> Vec<u8> {
    let bytes = match String::from_utf8(bytes) {
        Ok(s) => return s.into_bytes(),
        Err(e) => e.into_bytes(),
    };
    let mut out = Vec::with_capacity(bytes.len() + 64);
    let mut rest = &bytes[..];
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.extend_from_slice(s.as_bytes());
                return out;
            }
            Err(e) => {
                let (good, bad) = rest.split_at(e.valid_up_to());
                out.extend_from_slice(good);
                let n = e.error_len().unwrap_or(bad.len());
                for &b in &bad[..n] {
                    if (0x80..=0x9f).contains(&b) {
                        out.extend_from_slice(&[0x1b, b - 0x40]);
                    } else {
                        out.extend_from_slice("\u{fffd}".as_bytes());
                    }
                }
                rest = &bad[n..];
            }
        }
    }
}

/// `text` in pieces of about `about` bytes, each ending on a character
/// boundary: shorter when a character straddles the size, longer only when
/// one character is longer than `about`.
pub fn chunks(text: &str, about: usize) -> impl Iterator<Item = &str> {
    let about = about.max(1);
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = about.min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            end = rest.char_indices().nth(1).map_or(rest.len(), |(i, _)| i);
        }
        let (piece, tail) = rest.split_at(end);
        rest = tail;
        Some(piece)
    })
}

/// How `recovery-emit` was asked to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Args {
    pub seed: u64,
    pub bytes: u64,
    pub chunk: usize,
    pub pause: Duration,
}

impl Args {
    /// `<seed> <bytes> <chunk> <pause_ms>`, the program name left out.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
        let args: Vec<String> = args.into_iter().collect();
        let [seed, bytes, chunk, pause] = args.as_slice() else {
            return Err(format!(
                "expected <seed> <bytes> <chunk> <pause_ms>, got {} arguments",
                args.len()
            ));
        };
        let num = |name: &str, v: &str| {
            v.parse::<u64>()
                .map_err(|e| format!("{name} {v:?} is not a number: {e}"))
        };
        Ok(Args {
            seed: num("seed", seed)?,
            bytes: num("bytes", bytes)?,
            chunk: usize::try_from(num("chunk", chunk)?)
                .map_err(|_| format!("chunk {chunk} is too large"))?,
            pause: Duration::from_millis(num("pause_ms", pause)?),
        })
    }
}

/// Prints [`output`] for `args` to `out` in [`chunks`], flushing each and
/// sleeping `args.pause` between them. Returns once everything is written;
/// the binary then waits to be killed.
pub fn run(args: Args, out: &mut impl Write) -> io::Result<()> {
    let text = String::from_utf8(output(args.seed, args.bytes))
        .map_err(|e| io::Error::other(format!("output is not UTF-8: {e}")))?;
    for (i, piece) in chunks(&text, args.chunk).enumerate() {
        if i > 0 && !args.pause.is_zero() {
            std::thread::sleep(args.pause);
        }
        out.write_all(piece.as_bytes())?;
        out.flush()?;
    }
    Ok(())
}

/// Reads and drops whatever arrives on stdin, on a thread of its own, so
/// the answers a terminal sends to the queries in the output never fill a
/// pipe the program does not read.
pub fn drain_stdin() {
    std::thread::spawn(|| {
        let mut buf = [0u8; 4096];
        let mut stdin = io::stdin().lock();
        while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
    });
}

/// Turns on VT processing for stdout when it is a Windows console, so
/// escape sequences are interpreted rather than shown. Anywhere else, and
/// on a console that refuses, nothing changes.
pub fn enable_vt() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            STD_OUTPUT_HANDLE,
        };
        // SAFETY: GetStdHandle has no preconditions; the console calls take
        // that handle and a valid pointer to a local, and fail harmlessly on
        // a handle that is not a console.
        unsafe {
            let h = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(h, &mut mode) != 0 {
                SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_the_generated_data_and_utf8() {
        let out = output(3, 32 << 10);
        let log = Generator::log(3, Profile::round_trip().bytes(32 << 10));
        let mut data = Vec::new();
        for e in &log.entries {
            if let Record::Data { bytes, .. } = &e.rec {
                data.extend_from_slice(bytes);
            }
        }
        // The generator's only bytes that are not UTF-8 are C1 STs.
        let mut want = Vec::new();
        let mut sts = 0;
        for chunk in data.utf8_chunks() {
            want.extend_from_slice(chunk.valid().as_bytes());
            for &b in chunk.invalid() {
                assert_eq!(b, 0x9c, "a byte that is not UTF-8 nor ST");
                want.extend_from_slice(b"\x1b\\");
                sts += 1;
            }
        }
        assert!(sts > 0, "the seed exercises ST");
        assert!(std::str::from_utf8(&out).is_ok());
        assert_eq!(out, want);
        assert!(out.len() >= 32 << 10);
        assert_ne!(output(4, 32 << 10), out, "the seed matters");
    }

    #[test]
    fn to_utf8_keeps_characters_and_spells_out_c1() {
        let ok = "plain é 😀 \x1b[1m".as_bytes().to_vec();
        assert_eq!(to_utf8(ok.clone()), ok);
        assert_eq!(
            to_utf8(b"\x1bPq\x9cA\x9bm\xff\xc3\xa9\xe2\x82".to_vec()),
            // The cut-off character's lead byte, then its continuation, a C1
            // byte on its own.
            "\x1bPq\x1b\\A\x1b[m\u{fffd}é\u{fffd}\x1bB".as_bytes()
        );
    }

    #[test]
    fn chunks_end_on_character_boundaries_and_cover_everything() {
        let text = "aé😀b\u{301}c".repeat(50);
        for about in [1, 2, 3, 5, 7, 64, 10_000] {
            let pieces: Vec<&str> = chunks(&text, about).collect();
            assert_eq!(pieces.concat(), text, "about {about}");
            assert!(pieces.iter().all(|p| !p.is_empty()));
            // Each piece is at most `about`, or one character.
            assert!(
                pieces
                    .iter()
                    .all(|p| p.len() <= about || p.chars().count() == 1),
                "about {about}: {pieces:?}"
            );
        }
        assert_eq!(chunks("", 4).count(), 0);
    }

    #[test]
    fn run_writes_exactly_the_output() {
        let args = Args::parse(["9", "20000", "700", "0"].map(String::from)).unwrap();
        let mut out = Vec::new();
        run(args, &mut out).unwrap();
        assert_eq!(out, output(9, 20000));
    }

    #[test]
    fn args_say_what_is_wrong() {
        let a = Args::parse(["1", "2", "3", "4"].map(String::from)).unwrap();
        assert_eq!(a.pause, Duration::from_millis(4));
        let e = Args::parse(["1", "2"].map(String::from)).unwrap_err();
        assert!(e.contains("got 2 arguments"), "{e}");
        let e = Args::parse(["1", "x", "3", "4"].map(String::from)).unwrap_err();
        assert!(e.contains("bytes \"x\""), "{e}");
    }
}
