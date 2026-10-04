//! UTF-16 to UTF-8, for text that arrives from JavaScript.
//!
//! V8 converts a two-byte string to UTF-8 at about a millisecond per megabyte,
//! which was a third of what native analysis cost on an agent's output. Copying
//! the UTF-16 out is a memcpy, and terminal output is mostly ASCII, which this
//! moves eight units at a time.

/// Appends `src` to `dst` as UTF-8. A lone surrogate becomes U+FFFD, as V8's
/// own conversion does.
pub fn push_utf16(dst: &mut String, src: &[u16]) {
    let mut out = std::mem::take(dst).into_bytes();
    out.reserve(src.len());
    let mut i = 0;
    while i < src.len() {
        while let Some(block) = src.get(i..i + 8) {
            if block.iter().fold(0, |acc, &u| acc | u) >= 0x80 {
                break;
            }
            out.extend(block.iter().map(|&u| u as u8));
            i += 8;
        }
        let Some(&u) = src.get(i) else { break };
        i += 1;
        match u {
            0..=0x7f => out.push(u as u8),
            0x80..=0x7ff => {
                out.extend_from_slice(&[0xc0 | (u >> 6) as u8, 0x80 | (u & 0x3f) as u8])
            }
            0xd800..=0xdbff if matches!(src.get(i), Some(0xdc00..=0xdfff)) => {
                let c = 0x10000 + ((u32::from(u) - 0xd800) << 10) + (u32::from(src[i]) - 0xdc00);
                i += 1;
                out.extend_from_slice(&[
                    0xf0 | (c >> 18) as u8,
                    0x80 | ((c >> 12) & 0x3f) as u8,
                    0x80 | ((c >> 6) & 0x3f) as u8,
                    0x80 | (c & 0x3f) as u8,
                ]);
            }
            0xd800..=0xdfff => out.extend_from_slice("\u{fffd}".as_bytes()),
            _ => out.extend_from_slice(&[
                0xe0 | (u >> 12) as u8,
                0x80 | ((u >> 6) & 0x3f) as u8,
                0x80 | (u & 0x3f) as u8,
            ]),
        }
    }
    debug_assert!(std::str::from_utf8(&out).is_ok());
    // SAFETY: `out` started as a `String`'s bytes, and every arm above appends
    // one well-formed UTF-8 sequence: ASCII, a two- or three-byte sequence for
    // a non-surrogate unit, four bytes for a valid surrogate pair, or U+FFFD.
    // The tests compare it with `String::from_utf16_lossy` on random input.
    // Validating again here cost a third of the conversion.
    *dst = unsafe { String::from_utf8_unchecked(out) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(s: &[u16]) -> String {
        let mut out = String::from(">");
        push_utf16(&mut out, s);
        out
    }

    #[test]
    fn matches_std_on_valid_text() {
        for text in [
            "",
            "plain ascii, longer than one block of eight",
            "\x1b[38;5;174m⠸\x1b[39m Thinking… (0s · ↑ 0.0k tokens)",
            "é漢字😀 mixed 😀😀 end",
            "\u{7f}\u{80}\u{7ff}\u{800}\u{ffff}\u{10000}\u{10ffff}",
        ] {
            let units: Vec<u16> = text.encode_utf16().collect();
            assert_eq!(conv(&units), format!(">{text}"));
        }
    }

    #[test]
    fn replaces_lone_surrogates() {
        assert_eq!(conv(&[0x61, 0xd800, 0x62]), ">a\u{fffd}b");
        assert_eq!(conv(&[0xdc00]), ">\u{fffd}");
        assert_eq!(conv(&[0xd83d]), ">\u{fffd}");
        // A low surrogate before a high one is two lone ones.
        assert_eq!(conv(&[0xde00, 0xd83d]), ">\u{fffd}\u{fffd}");
    }

    #[test]
    fn agrees_with_std_lossy_on_noise() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..2000 {
            let units: Vec<u16> = (0..24)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    [0x41, 0x7f, 0xe9, 0x2838, 0xd83d, 0xde00, 0xdc00, 0xfffd][(seed % 8) as usize]
                })
                .collect();
            assert_eq!(
                conv(&units),
                format!(">{}", String::from_utf16_lossy(&units))
            );
        }
    }
}
