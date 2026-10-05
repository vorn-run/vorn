//! A person's typing, told apart from what a terminal sends by itself.
//!
//! A bytes client (xterm.js in the renderer, the web client, the phone)
//! writes everything its terminal produces: keys and pastes, but also focus
//! reports when a program turned them on (mode 1004) and mouse reports when
//! it asked for those. Focusing a pane or scrolling it on the phone must not
//! take the size (TP §10: looking never resizes), so those reports are not
//! input here. Clicks are: a click in a full-screen program is work done
//! there, like a key.

/// Whether `bytes`, written by a client, hold anything a person did beyond
/// focusing or scrolling: anything left once focus reports and mouse motion
/// and wheel reports are taken out.
pub fn typed(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        match report_len(&bytes[i..]) {
            Some(n) => i += n,
            None => return true,
        }
    }
    false
}

/// The length of the report `b` starts with, if it starts with one that is
/// not input: `ESC [ I`, `ESC [ O`, an X10 mouse report `ESC [ M b x y` or
/// an SGR one `ESC [ < b ; x ; y M|m`, for motion or the wheel.
fn report_len(b: &[u8]) -> Option<usize> {
    let rest = b.strip_prefix(b"\x1b[")?;
    match rest.first()? {
        b'I' | b'O' => Some(3),
        b'M' => {
            let &[_, cb, _, _, ..] = rest else {
                return None;
            };
            passive(u32::from(cb.checked_sub(32)?)).then_some(6)
        }
        b'<' => {
            let end = rest.iter().position(|&c| c == b'M' || c == b'm')?;
            let params = std::str::from_utf8(&rest[1..end]).ok()?;
            let mut parts = params.split(';');
            let button: u32 = parts.next()?.parse().ok()?;
            let numbers = parts.filter(|p| p.parse::<u32>().is_ok()).count();
            (numbers == 2 && passive(button)).then_some(2 + end + 1)
        }
        _ => None,
    }
}

/// Whether a mouse report's button byte is the wheel (64 and up) or motion
/// (32 set), rather than a press or a release.
fn passive(button: u32) -> bool {
    button & 64 != 0 || button & 32 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_pastes_and_clicks_are_typing() {
        for b in [
            &b"ls\r"[..],
            b"\x1b[A",
            b"\x1b[200~text\x1b[201~",
            b"\x1b[<0;10;5M",
            b"\x1b[<0;10;5m",
            b"\x1b[M #%",
            // A focus report with a key after it.
            b"\x1b[Ix",
        ] {
            assert!(typed(b), "{:?}", String::from_utf8_lossy(b));
        }
    }

    #[test]
    fn focus_scroll_and_motion_are_not() {
        for b in [
            &b""[..],
            b"\x1b[I",
            b"\x1b[O",
            b"\x1b[I\x1b[O\x1b[I",
            b"\x1b[<64;10;5M",
            b"\x1b[<65;1;1M\x1b[<65;1;1M",
            b"\x1b[<35;80;24M",
            b"\x1b[M`!!",
            b"\x1b[Ma!!",
            b"\x1b[MC!!",
        ] {
            assert!(!typed(b), "{:?}", String::from_utf8_lossy(b));
        }
    }

    #[test]
    fn a_report_cut_short_is_typing_rather_than_ignored() {
        // Half a report cannot be told from a key; it never hides a key.
        for b in [&b"\x1b["[..], b"\x1b[<64;1", b"\x1b[M`", b"\x1b[<64;x;1M"] {
            assert!(typed(b), "{:?}", String::from_utf8_lossy(b));
        }
    }
}
