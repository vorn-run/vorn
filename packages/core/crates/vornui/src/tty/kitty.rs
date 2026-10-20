//! The kitty graphics protocol's commands as exact bytes: sending an image
//! (inline in chunks, or as a temporary file the terminal reads), placing it
//! at the cursor or as cells of placeholder text, and deleting it; each
//! wrapped for tmux's passthrough when the terminal is behind tmux.

use std::io::Write;
use std::path::Path;

/// Pixels per inline chunk: 3072 bytes are 4096 in base64, the most a
/// chunk may carry, and a multiple of 3 so every chunk but the last is
/// unpadded.
const CHUNK: usize = 3072;

/// How each escape sequence reaches the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    /// Written as is.
    Direct,
    /// Inside tmux: passed through to the terminal tmux runs in, which
    /// needs `set -g allow-passthrough on`.
    Tmux,
}

impl Wrap {
    /// Tmux when the process runs inside it.
    pub fn from_env() -> Wrap {
        if std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()) {
            Wrap::Tmux
        } else {
            Wrap::Direct
        }
    }

    /// Appends `seq` to `out` as this wrap sends it: tmux's passthrough
    /// doubles every ESC inside.
    pub fn put(self, out: &mut Vec<u8>, seq: &[u8]) {
        match self {
            Wrap::Direct => out.extend_from_slice(seq),
            Wrap::Tmux => {
                out.extend_from_slice(b"\x1bPtmux;");
                for &b in seq {
                    if b == 0x1b {
                        out.push(0x1b);
                    }
                    out.push(b);
                }
                out.extend_from_slice(b"\x1b\\");
            }
        }
    }
}

/// An image's pixels: tightly packed rows, 3 or 4 bytes a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Rgb,
    Rgba,
}

impl Format {
    fn code(self) -> u8 {
        match self {
            Format::Rgb => 24,
            Format::Rgba => 32,
        }
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Format::Rgb => 3,
            Format::Rgba => 4,
        }
    }
}

/// Where a sent image shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// At the cursor, `offset` pixels into its cell, at depth `z`; the
    /// cursor stays where it is.
    Cursor { offset: (u32, u32), z: i32 },
    /// Wherever placeholder cells name it, scaled to `cols` by `rows`
    /// cells: what tmux keeps on screen, since to it they are text.
    Cells { cols: u32, rows: u32 },
}

/// How an image's pixels travel.
#[derive(Debug, Clone, Copy)]
pub enum Medium<'a> {
    /// Inline in base64 chunks; `zlib` when the bytes are deflated.
    Direct { zlib: bool },
    /// In a temporary file on the terminal's machine, which it deletes
    /// once read: no copy through the tty.
    File(&'a Path),
}

/// Sends image `id` of `size` pixels and shows it per `show`. `data` is
/// the pixels (deflated when [`Medium::Direct`] says so); with
/// [`Medium::File`] it is ignored, the file holds them.
pub fn transmit(
    out: &mut Vec<u8>,
    wrap: Wrap,
    id: u32,
    (size, format): ((u32, u32), Format),
    show: Show,
    medium: Medium<'_>,
    data: &[u8],
) {
    let mut head = Vec::with_capacity(96);
    let _ = write!(
        head,
        "\x1b_Ga=T,f={},s={},v={},i={id},q=2",
        format.code(),
        size.0,
        size.1
    );
    match show {
        Show::Cursor { offset, z } => {
            let _ = write!(head, ",C=1,z={z}");
            if offset.0 > 0 {
                let _ = write!(head, ",X={}", offset.0);
            }
            if offset.1 > 0 {
                let _ = write!(head, ",Y={}", offset.1);
            }
        }
        Show::Cells { cols, rows } => {
            let _ = write!(head, ",U=1,c={cols},r={rows}");
        }
    }
    match medium {
        Medium::File(path) => {
            head.extend_from_slice(b",t=t;");
            base64_into(path.as_os_str().as_encoded_bytes(), &mut head);
            head.extend_from_slice(b"\x1b\\");
            wrap.put(out, &head);
        }
        Medium::Direct { zlib } => {
            if zlib {
                head.extend_from_slice(b",o=z");
            }
            let mut seq = Vec::with_capacity(4096 + 32);
            let mut chunks = data.chunks(CHUNK).peekable();
            let mut first = true;
            // An empty image still needs its one command.
            if chunks.peek().is_none() {
                head.extend_from_slice(b";\x1b\\");
                wrap.put(out, &head);
                return;
            }
            while let Some(chunk) = chunks.next() {
                seq.clear();
                if first {
                    seq.extend_from_slice(&head);
                    seq.push(b',');
                    first = false;
                } else {
                    seq.extend_from_slice(b"\x1b_G");
                }
                let more = u8::from(chunks.peek().is_some());
                let _ = write!(seq, "m={more};");
                base64_into(chunk, &mut seq);
                seq.extend_from_slice(b"\x1b\\");
                wrap.put(out, &seq);
            }
        }
    }
}

/// Deletes image `id` and frees its pixels.
pub fn delete(out: &mut Vec<u8>, wrap: Wrap, id: u32) {
    let mut seq = Vec::with_capacity(32);
    let _ = write!(seq, "\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
    wrap.put(out, &seq);
}

/// Deletes every image on screen and frees their pixels.
pub fn delete_all(out: &mut Vec<u8>, wrap: Wrap) {
    wrap.put(out, b"\x1b_Ga=d,d=A,q=2\x1b\\");
}

/// The query whose answer says the terminal shows images: a 1x1 image
/// sent inline but not kept.
pub fn query_direct(out: &mut Vec<u8>, wrap: Wrap, id: u32) {
    let mut seq = Vec::new();
    let _ = write!(seq, "\x1b_Gi={id},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\");
    wrap.put(out, &seq);
}

/// The query whose answer says the terminal reads temporary files from
/// `path` (which holds a 1x1 RGB image): only when it runs on this machine.
pub fn query_file(out: &mut Vec<u8>, wrap: Wrap, id: u32, path: &Path) {
    let mut seq = Vec::new();
    let _ = write!(seq, "\x1b_Gi={id},s=1,v=1,a=q,t=t,f=24;");
    base64_into(path.as_os_str().as_encoded_bytes(), &mut seq);
    seq.extend_from_slice(b"\x1b\\");
    wrap.put(out, &seq);
}

/// Writes the placeholder cells that show image `id` (placed with
/// [`Show::Cells`]) as a `cols` by `rows` block whose top-left cell is
/// `at` (0-based column, row). The image's row is named on each line's
/// first cell; the cells after it count on by themselves.
pub fn placeholders(out: &mut Vec<u8>, id: u32, at: (u32, u32), cols: u32, rows: u32) {
    let [_, r, g, b] = id.to_be_bytes();
    let _ = write!(out, "\x1b[38;2;{r};{g};{b}m");
    let mut buf = [0u8; 4];
    for row in 0..rows.min(DIACRITICS.len() as u32) {
        let _ = write!(out, "\x1b[{};{}H", at.1 + row + 1, at.0 + 1);
        for col in 0..cols {
            out.extend_from_slice(PLACEHOLDER.encode_utf8(&mut buf).as_bytes());
            if col == 0 {
                out.extend_from_slice(DIACRITICS[row as usize].encode_utf8(&mut buf).as_bytes());
                out.extend_from_slice(DIACRITICS[0].encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.extend_from_slice(b"\x1b[39m");
}

/// The most rows placeholder cells can number; columns count on from the first.
pub const MAX_PLACEHOLDER_ROWS: u32 = DIACRITICS.len() as u32;

/// The character placeholder cells hold.
const PLACEHOLDER: char = '\u{10EEEE}';

/// Standard base64 with padding, appended to `out`: how the protocol
/// carries pixels and paths.
pub fn base64_into(bytes: &[u8], out: &mut Vec<u8>) {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    out.reserve(bytes.len().div_ceil(3) * 4);
    let mut whole = bytes.chunks_exact(3);
    for c in &mut whole {
        let n = u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2]);
        out.extend_from_slice(&[
            ABC[(n >> 18) as usize & 63],
            ABC[(n >> 12) as usize & 63],
            ABC[(n >> 6) as usize & 63],
            ABC[n as usize & 63],
        ]);
    }
    match *whole.remainder() {
        [a] => {
            let n = u32::from(a) << 16;
            out.extend_from_slice(&[ABC[(n >> 18) as usize & 63], ABC[(n >> 12) as usize & 63]]);
            out.extend_from_slice(b"==");
        }
        [a, b] => {
            let n = u32::from(a) << 16 | u32::from(b) << 8;
            out.extend_from_slice(&[
                ABC[(n >> 18) as usize & 63],
                ABC[(n >> 12) as usize & 63],
                ABC[(n >> 6) as usize & 63],
                b'=',
            ]);
        }
        _ => {}
    }
}

/// The combining marks that number a placeholder cell's row and column,
/// in the order the graphics protocol gives them.
#[rustfmt::skip]
const DIACRITICS: [char; 297] = [
    '\u{305}', '\u{30d}', '\u{30e}', '\u{310}', '\u{312}', '\u{33d}', '\u{33e}', '\u{33f}',
    '\u{346}', '\u{34a}', '\u{34b}', '\u{34c}', '\u{350}', '\u{351}', '\u{352}', '\u{357}',
    '\u{35b}', '\u{363}', '\u{364}', '\u{365}', '\u{366}', '\u{367}', '\u{368}', '\u{369}',
    '\u{36a}', '\u{36b}', '\u{36c}', '\u{36d}', '\u{36e}', '\u{36f}', '\u{483}', '\u{484}',
    '\u{485}', '\u{486}', '\u{487}', '\u{592}', '\u{593}', '\u{594}', '\u{595}', '\u{597}',
    '\u{598}', '\u{599}', '\u{59c}', '\u{59d}', '\u{59e}', '\u{59f}', '\u{5a0}', '\u{5a1}',
    '\u{5a8}', '\u{5a9}', '\u{5ab}', '\u{5ac}', '\u{5af}', '\u{5c4}', '\u{610}', '\u{611}',
    '\u{612}', '\u{613}', '\u{614}', '\u{615}', '\u{616}', '\u{617}', '\u{657}', '\u{658}',
    '\u{659}', '\u{65a}', '\u{65b}', '\u{65d}', '\u{65e}', '\u{6d6}', '\u{6d7}', '\u{6d8}',
    '\u{6d9}', '\u{6da}', '\u{6db}', '\u{6dc}', '\u{6df}', '\u{6e0}', '\u{6e1}', '\u{6e2}',
    '\u{6e4}', '\u{6e7}', '\u{6e8}', '\u{6eb}', '\u{6ec}', '\u{730}', '\u{732}', '\u{733}',
    '\u{735}', '\u{736}', '\u{73a}', '\u{73d}', '\u{73f}', '\u{740}', '\u{741}', '\u{743}',
    '\u{745}', '\u{747}', '\u{749}', '\u{74a}', '\u{7eb}', '\u{7ec}', '\u{7ed}', '\u{7ee}',
    '\u{7ef}', '\u{7f0}', '\u{7f1}', '\u{7f3}', '\u{816}', '\u{817}', '\u{818}', '\u{819}',
    '\u{81b}', '\u{81c}', '\u{81d}', '\u{81e}', '\u{81f}', '\u{820}', '\u{821}', '\u{822}',
    '\u{823}', '\u{825}', '\u{826}', '\u{827}', '\u{829}', '\u{82a}', '\u{82b}', '\u{82c}',
    '\u{82d}', '\u{951}', '\u{953}', '\u{954}', '\u{f82}', '\u{f83}', '\u{f86}', '\u{f87}',
    '\u{135d}', '\u{135e}', '\u{135f}', '\u{17dd}', '\u{193a}', '\u{1a17}', '\u{1a75}', '\u{1a76}',
    '\u{1a77}', '\u{1a78}', '\u{1a79}', '\u{1a7a}', '\u{1a7b}', '\u{1a7c}', '\u{1b6b}', '\u{1b6d}',
    '\u{1b6e}', '\u{1b6f}', '\u{1b70}', '\u{1b71}', '\u{1b72}', '\u{1b73}', '\u{1cd0}', '\u{1cd1}',
    '\u{1cd2}', '\u{1cda}', '\u{1cdb}', '\u{1ce0}', '\u{1dc0}', '\u{1dc1}', '\u{1dc3}', '\u{1dc4}',
    '\u{1dc5}', '\u{1dc6}', '\u{1dc7}', '\u{1dc8}', '\u{1dc9}', '\u{1dcb}', '\u{1dcc}', '\u{1dd1}',
    '\u{1dd2}', '\u{1dd3}', '\u{1dd4}', '\u{1dd5}', '\u{1dd6}', '\u{1dd7}', '\u{1dd8}', '\u{1dd9}',
    '\u{1dda}', '\u{1ddb}', '\u{1ddc}', '\u{1ddd}', '\u{1dde}', '\u{1ddf}', '\u{1de0}', '\u{1de1}',
    '\u{1de2}', '\u{1de3}', '\u{1de4}', '\u{1de5}', '\u{1de6}', '\u{1dfe}', '\u{20d0}', '\u{20d1}',
    '\u{20d4}', '\u{20d5}', '\u{20d6}', '\u{20d7}', '\u{20db}', '\u{20dc}', '\u{20e1}', '\u{20e7}',
    '\u{20e9}', '\u{20f0}', '\u{2cef}', '\u{2cf0}', '\u{2cf1}', '\u{2de0}', '\u{2de1}', '\u{2de2}',
    '\u{2de3}', '\u{2de4}', '\u{2de5}', '\u{2de6}', '\u{2de7}', '\u{2de8}', '\u{2de9}', '\u{2dea}',
    '\u{2deb}', '\u{2dec}', '\u{2ded}', '\u{2dee}', '\u{2def}', '\u{2df0}', '\u{2df1}', '\u{2df2}',
    '\u{2df3}', '\u{2df4}', '\u{2df5}', '\u{2df6}', '\u{2df7}', '\u{2df8}', '\u{2df9}', '\u{2dfa}',
    '\u{2dfb}', '\u{2dfc}', '\u{2dfd}', '\u{2dfe}', '\u{2dff}', '\u{a66f}', '\u{a67c}', '\u{a67d}',
    '\u{a6f0}', '\u{a6f1}', '\u{a8e0}', '\u{a8e1}', '\u{a8e2}', '\u{a8e3}', '\u{a8e4}', '\u{a8e5}',
    '\u{a8e6}', '\u{a8e7}', '\u{a8e8}', '\u{a8e9}', '\u{a8ea}', '\u{a8eb}', '\u{a8ec}', '\u{a8ed}',
    '\u{a8ee}', '\u{a8ef}', '\u{a8f0}', '\u{a8f1}', '\u{aab0}', '\u{aab2}', '\u{aab3}', '\u{aab7}',
    '\u{aab8}', '\u{aabe}', '\u{aabf}', '\u{aac1}', '\u{fe20}', '\u{fe21}', '\u{fe22}', '\u{fe23}',
    '\u{fe24}', '\u{fe25}', '\u{fe26}', '\u{10a0f}', '\u{10a38}', '\u{1d185}', '\u{1d186}',
    '\u{1d187}', '\u{1d188}', '\u{1d189}', '\u{1d1aa}', '\u{1d1ab}', '\u{1d1ac}', '\u{1d1ad}',
    '\u{1d242}', '\u{1d243}', '\u{1d244}',
];

#[cfg(test)]
mod tests {
    use super::*;

    fn text(b: &[u8]) -> String {
        String::from_utf8(b.to_vec()).expect("ascii")
    }

    #[test]
    fn base64_is_the_standard_alphabet_with_padding() {
        let enc = |b: &[u8]| {
            let mut out = Vec::new();
            base64_into(b, &mut out);
            text(&out)
        };
        assert_eq!(enc(b""), "");
        assert_eq!(enc(b"f"), "Zg==");
        assert_eq!(enc(b"fo"), "Zm8=");
        assert_eq!(enc(b"foo"), "Zm9v");
        assert_eq!(enc(b"foobar"), "Zm9vYmFy");
        assert_eq!(enc(&[0xff, 0xfe, 0xfd, 0x00]), "//79AA==");
    }

    #[test]
    fn a_small_image_goes_in_one_command_placed_at_the_cursor() {
        let mut out = Vec::new();
        transmit(
            &mut out,
            Wrap::Direct,
            7,
            ((1, 1), Format::Rgb),
            Show::Cursor {
                offset: (3, 0),
                z: 2,
            },
            Medium::Direct { zlib: false },
            &[1, 2, 3],
        );
        assert_eq!(
            text(&out),
            "\x1b_Ga=T,f=24,s=1,v=1,i=7,q=2,C=1,z=2,X=3,m=0;AQID\x1b\\"
        );
    }

    #[test]
    fn a_large_image_is_chunked_at_4096_base64_bytes() {
        let data = vec![0u8; CHUNK * 2 + 3];
        let mut out = Vec::new();
        transmit(
            &mut out,
            Wrap::Direct,
            9,
            ((1, (CHUNK * 2 + 3) as u32 / 3), Format::Rgb),
            Show::Cursor {
                offset: (0, 5),
                z: -1,
            },
            Medium::Direct { zlib: true },
            &data,
        );
        let s = text(&out);
        let cmds: Vec<&str> = s.split("\x1b\\").filter(|c| !c.is_empty()).collect();
        assert_eq!(cmds.len(), 3);
        assert!(cmds[0].starts_with("\x1b_Ga=T,f=24,s=1,v=2049,i=9,q=2,C=1,z=-1,Y=5,o=z,m=1;"));
        assert_eq!(cmds[0].split(';').nth(1).map(str::len), Some(4096));
        assert!(cmds[1].starts_with("\x1b_Gm=1;"));
        assert_eq!(cmds[2], "\x1b_Gm=0;AAAA");
    }

    #[test]
    fn a_file_sends_its_path_not_its_pixels() {
        let mut out = Vec::new();
        transmit(
            &mut out,
            Wrap::Direct,
            3,
            ((2, 2), Format::Rgb),
            Show::Cells { cols: 1, rows: 1 },
            Medium::File(Path::new("/tmp/a")),
            &[0; 12],
        );
        assert_eq!(
            text(&out),
            "\x1b_Ga=T,f=24,s=2,v=2,i=3,q=2,U=1,c=1,r=1,t=t;L3RtcC9h\x1b\\"
        );
    }

    #[test]
    fn tmux_passes_each_command_through_with_its_escapes_doubled() {
        let mut out = Vec::new();
        delete(&mut out, Wrap::Tmux, 5);
        assert_eq!(
            text(&out),
            "\x1bPtmux;\x1b\x1b_Ga=d,d=I,i=5,q=2\x1b\x1b\\\x1b\\"
        );
        let mut out = Vec::new();
        transmit(
            &mut out,
            Wrap::Tmux,
            1,
            ((1, 2049), Format::Rgb),
            Show::Cells { cols: 1, rows: 1 },
            Medium::Direct { zlib: false },
            &vec![0; CHUNK + 3],
        );
        assert_eq!(
            text(&out).matches("\x1bPtmux;").count(),
            2,
            "one wrap a chunk"
        );
    }

    #[test]
    fn queries_ask_for_an_answer() {
        let mut out = Vec::new();
        query_direct(&mut out, Wrap::Direct, 31);
        query_file(&mut out, Wrap::Direct, 32, Path::new("/tmp/a"));
        delete_all(&mut out, Wrap::Direct);
        assert_eq!(
            text(&out),
            "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\
             \x1b_Gi=32,s=1,v=1,a=q,t=t,f=24;L3RtcC9h\x1b\\\
             \x1b_Ga=d,d=A,q=2\x1b\\"
        );
    }

    #[test]
    fn placeholder_lines_name_their_row_and_count_on() {
        let mut out = Vec::new();
        placeholders(&mut out, 0x01_02_03, (4, 1), 2, 2);
        assert_eq!(
            String::from_utf8(out).expect("utf-8"),
            "\x1b[38;2;1;2;3m\
             \x1b[2;5H\u{10EEEE}\u{305}\u{305}\u{10EEEE}\
             \x1b[3;5H\u{10EEEE}\u{30d}\u{305}\u{10EEEE}\
             \x1b[39m"
        );
    }
}
