//! The row cell encoding, `row_fmt = 1` (Terminal State Protocol §6).
//!
//! ```text
//! cells     := run*                    trailing blank default-style cells are omitted
//! run       := varint style_id  varint link_id  varint (n << 1 | kind)  body
//! kind 0    := ascii run: n bytes, one per cell, each 0x20..0x7E, or 0x00 for an empty cell
//! kind 1    := general run: cell{n}
//! cell      := u8 head  [varint ext_len]  utf8[len]
//! head      := bits 0-5 len (0 = empty, 1..62 = byte length, 63 = escape: the length follows as a varint)
//!            | bit 6 wide (the spacer tail is implied and not sent)
//!            | bit 7 reserved, 0 in row_fmt 1
//! ```
//!
//! Most terminal text is printable ASCII in a few styles, which costs one byte
//! a cell. Anything else costs a head byte plus its UTF-8.

/// The cell layout this module writes and reads.
pub const ROW_FMT: u8 = 1;
/// The longest grapheme cluster a cell carries. Anything longer arrives as
/// U+FFFD, so a hostile program cannot make one cell arbitrarily large.
pub const MAX_CLUSTER_BYTES: usize = 4096;

const LEN_ESCAPE: u8 = 63;
const WIDE: u8 = 0x40;
const RESERVED: u8 = 0x80;
const REPLACEMENT: &str = "\u{FFFD}";

/// One cell: its grapheme cluster, empty for a cell nothing was written to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub text: String,
    /// Takes two columns; the spacer after it is implied.
    pub wide: bool,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            wide: false,
        }
    }

    pub fn wide(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            wide: true,
        }
    }

    fn ascii(&self) -> Option<u8> {
        if self.wide {
            return None;
        }
        match self.text.as_bytes() {
            [] => Some(0),
            [b @ 0x20..=0x7e] => Some(*b),
            _ => None,
        }
    }

    fn is_blank(&self) -> bool {
        !self.wide && (self.text.is_empty() || self.text == " ")
    }
}

/// Cells that share a style and a link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Run {
    pub style: u32,
    pub link: u32,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes end inside a run.
    Truncated,
    /// A varint longer than a u64.
    Overlong,
    /// The reserved head bit is set.
    Reserved,
    /// A cell's bytes are not UTF-8.
    NotUtf8,
    /// An ascii run holds a byte outside 0x20..0x7E and 0x00.
    NotAscii,
    /// A count or length larger than the bytes that could hold it.
    TooLong,
}

/// Encode a row. Trailing blank cells in the default style with no link are
/// left out; the reader knows the row's width from the frame.
pub fn encode(runs: &[Run]) -> Vec<u8> {
    let mut runs = runs.to_vec();
    while let Some(last) = runs.last_mut() {
        if last.style != 0 || last.link != 0 {
            break;
        }
        while last.cells.last().is_some_and(Cell::is_blank) {
            last.cells.pop();
        }
        if !last.cells.is_empty() {
            break;
        }
        runs.pop();
    }

    let mut out = Vec::new();
    for run in runs.iter().filter(|r| !r.cells.is_empty()) {
        varint(&mut out, u64::from(run.style));
        varint(&mut out, u64::from(run.link));
        let n = run.cells.len() as u64;
        let ascii: Option<Vec<u8>> = run.cells.iter().map(Cell::ascii).collect();
        match ascii {
            Some(bytes) => {
                varint(&mut out, n << 1);
                out.extend_from_slice(&bytes);
            }
            None => {
                varint(&mut out, (n << 1) | 1);
                for cell in &run.cells {
                    let text = if cell.text.len() > MAX_CLUSTER_BYTES {
                        REPLACEMENT
                    } else {
                        cell.text.as_str()
                    };
                    let wide = if cell.wide { WIDE } else { 0 };
                    let len = text.len();
                    if len < LEN_ESCAPE as usize {
                        out.push(len as u8 | wide);
                    } else {
                        out.push(LEN_ESCAPE | wide);
                        varint(&mut out, len as u64);
                    }
                    out.extend_from_slice(text.as_bytes());
                }
            }
        }
    }
    out
}

/// Decode a row. Malformed input is an error, never a panic, and never more
/// cells than the input could have encoded.
pub fn decode(mut bytes: &[u8]) -> Result<Vec<Run>, DecodeError> {
    let mut runs = Vec::new();
    while !bytes.is_empty() {
        let style = u32::try_from(read_varint(&mut bytes)?).map_err(|_| DecodeError::TooLong)?;
        let link = u32::try_from(read_varint(&mut bytes)?).map_err(|_| DecodeError::TooLong)?;
        let head = read_varint(&mut bytes)?;
        let n = usize::try_from(head >> 1).map_err(|_| DecodeError::TooLong)?;
        // Every cell takes at least one byte, so a count past the rest is a lie.
        if n > bytes.len() {
            return Err(DecodeError::TooLong);
        }
        let mut cells = Vec::with_capacity(n);
        if head & 1 == 0 {
            for &b in &bytes[..n] {
                cells.push(match b {
                    0 => Cell::default(),
                    0x20..=0x7e => Cell::new((b as char).to_string()),
                    _ => return Err(DecodeError::NotAscii),
                });
            }
            bytes = &bytes[n..];
        } else {
            for _ in 0..n {
                let (&h, rest) = bytes.split_first().ok_or(DecodeError::Truncated)?;
                bytes = rest;
                if h & RESERVED != 0 {
                    return Err(DecodeError::Reserved);
                }
                let len = match h & LEN_ESCAPE {
                    LEN_ESCAPE => usize::try_from(read_varint(&mut bytes)?)
                        .map_err(|_| DecodeError::TooLong)?,
                    short => short as usize,
                };
                // The encoder never sends a longer cluster.
                if len > MAX_CLUSTER_BYTES {
                    return Err(DecodeError::TooLong);
                }
                if len > bytes.len() {
                    return Err(DecodeError::Truncated);
                }
                let text = std::str::from_utf8(&bytes[..len]).map_err(|_| DecodeError::NotUtf8)?;
                bytes = &bytes[len..];
                cells.push(Cell {
                    text: text.to_owned(),
                    wide: h & WIDE != 0,
                });
            }
        }
        runs.push(Run { style, link, cells });
    }
    Ok(runs)
}

/// Encodes a row one cell at a time into a buffer the caller keeps, with
/// the bytes [`encode`] would write for the same cells grouped into runs of
/// one style and link.
///
/// The hot path of vornd's render update: no allocation per cell or per row
/// once the buffers have grown to a row's size. A run is kept as an ascii
/// body while every cell fits one, and turned into a general body at the
/// first that does not.
#[derive(Debug, Default)]
pub struct RowWriter {
    /// The current run: style and link, or none before the first cell.
    run: Option<(u32, u32)>,
    cells: u64,
    /// The run as an ascii body, while every cell so far fits one.
    ascii: Vec<u8>,
    all_ascii: bool,
    /// The run as a general body, once a cell did not fit an ascii one.
    general: Vec<u8>,
    /// Where a streak of blank cells at the end of a default run began:
    /// (cells, ascii length, general length), dropped by `finish`.
    blanks: Option<(u64, usize, usize)>,
}

impl RowWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one cell. `text` longer than [`MAX_CLUSTER_BYTES`] is sent as
    /// U+FFFD, as [`encode`] does.
    pub fn cell(&mut self, out: &mut Vec<u8>, style: u32, link: u32, text: &str, wide: bool) {
        if self.run != Some((style, link)) {
            self.end_run(out);
            self.run = Some((style, link));
        }
        let text = if text.len() > MAX_CLUSTER_BYTES {
            REPLACEMENT
        } else {
            text
        };
        let blank = !wide && (text.is_empty() || text == " ");
        if blank && style == 0 && link == 0 {
            self.blanks
                .get_or_insert((self.cells, self.ascii.len(), self.general.len()));
        } else {
            self.blanks = None;
        }
        self.cells += 1;
        if self.all_ascii {
            match (wide, text.as_bytes()) {
                (false, []) => {
                    self.ascii.push(0);
                    return;
                }
                (false, [b @ 0x20..=0x7e]) => {
                    self.ascii.push(*b);
                    return;
                }
                _ => {
                    // The run so far, as general cells: an empty one is a
                    // zero head, a character a head of 1 and its byte.
                    self.all_ascii = false;
                    for &b in &self.ascii {
                        match b {
                            0 => self.general.push(0),
                            b => self.general.extend_from_slice(&[1, b]),
                        }
                    }
                    if let Some(at) = &mut self.blanks {
                        at.2 = self.ascii[..at.1]
                            .iter()
                            .map(|&b| if b == 0 { 1 } else { 2 })
                            .sum();
                    }
                }
            }
        }
        let flag = if wide { WIDE } else { 0 };
        let len = text.len();
        if len < LEN_ESCAPE as usize {
            self.general.push(len as u8 | flag);
        } else {
            self.general.push(LEN_ESCAPE | flag);
            varint(&mut self.general, len as u64);
        }
        self.general.extend_from_slice(text.as_bytes());
    }

    /// Adds one printable ASCII cell (0x20 to 0x7E): what [`RowWriter::cell`]
    /// does with it, for the most common cell there is.
    pub fn ascii(&mut self, out: &mut Vec<u8>, style: u32, link: u32, byte: u8) {
        if !(self.all_ascii && self.run == Some((style, link))) || byte == b' ' {
            let buf = [byte];
            self.cell(
                out,
                style,
                link,
                std::str::from_utf8(&buf).unwrap_or("?"),
                false,
            );
            return;
        }
        self.blanks = None;
        self.cells += 1;
        self.ascii.push(byte);
    }

    /// Ends the row: appends its last run to `out`, without trailing blank
    /// cells in the default style, and readies the writer for the next row.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if let Some((cells, ascii, general)) = self.blanks.take() {
            self.cells = cells;
            self.ascii.truncate(ascii);
            self.general.truncate(general);
        }
        self.end_run(out);
        self.run = None;
    }

    fn end_run(&mut self, out: &mut Vec<u8>) {
        self.blanks = None;
        if let Some((style, link)) = self.run {
            if self.cells > 0 {
                varint(out, u64::from(style));
                varint(out, u64::from(link));
                if self.all_ascii {
                    varint(out, self.cells << 1);
                    out.extend_from_slice(&self.ascii);
                } else {
                    varint(out, (self.cells << 1) | 1);
                    out.extend_from_slice(&self.general);
                }
            }
        }
        self.cells = 0;
        self.ascii.clear();
        self.general.clear();
        self.all_ascii = true;
    }
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_varint(bytes: &mut &[u8]) -> Result<u64, DecodeError> {
    let mut v = 0u64;
    for i in 0..10 {
        let (&b, rest) = bytes.split_first().ok_or(DecodeError::Truncated)?;
        *bytes = rest;
        if i == 9 && b > 1 {
            return Err(DecodeError::Overlong);
        }
        v |= u64::from(b & 0x7f) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(DecodeError::Overlong)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(style: u32, cells: Vec<Cell>) -> Run {
        Run {
            style,
            link: 0,
            cells,
        }
    }

    fn ascii(s: &str) -> Vec<Cell> {
        s.chars().map(|c| Cell::new(c.to_string())).collect()
    }

    #[test]
    fn ascii_costs_a_byte_a_cell() {
        let row = vec![run(3, ascii("hello world"))];
        let bytes = encode(&row);
        assert_eq!(bytes.len(), 3 + 11);
        assert_eq!(decode(&bytes).unwrap(), row);
    }

    #[test]
    fn trailing_default_blanks_are_left_out() {
        let mut cells = ascii("$ ls");
        cells.extend(std::iter::repeat_n(Cell::default(), 76));
        let row = vec![run(0, cells)];
        assert_eq!(decode(&encode(&row)).unwrap(), vec![run(0, ascii("$ ls"))]);
        // Blanks with a background are kept.
        let styled = vec![run(0, ascii("x")), run(4, ascii("   "))];
        assert_eq!(decode(&encode(&styled)).unwrap(), styled);
    }

    #[test]
    fn wide_and_combining_cells_round_trip() {
        let row = vec![
            run(0, ascii("ab")),
            run(
                1,
                vec![
                    Cell::wide("日"),
                    Cell::new("e\u{301}"),
                    Cell::wide("👩‍👩‍👧"),
                    Cell::default(),
                ],
            ),
        ];
        assert_eq!(decode(&encode(&row)).unwrap(), row);
    }

    /// TP-T25: clusters of 62, 63, 64, 255 and 4,096 bytes round-trip; one of
    /// 4,097 arrives as U+FFFD.
    #[test]
    fn long_graphemes() {
        for len in [62usize, 63, 64, 255, 4096] {
            let text = format!("a{}", "\u{301}".repeat((len - 1) / 2));
            let text = if text.len() < len {
                format!("{text}b")
            } else {
                text
            };
            assert_eq!(text.len(), len);
            let row = vec![run(0, vec![Cell::new(text.clone()), Cell::new("z")])];
            assert_eq!(decode(&encode(&row)).unwrap(), row, "{len} bytes");
        }
        let long = format!("a{}", "\u{301}".repeat(2048));
        assert_eq!(long.len(), 4097);
        let out = decode(&encode(&[run(0, vec![Cell::new(long)])])).unwrap();
        assert_eq!(out[0].cells[0].text, REPLACEMENT);
    }

    /// TP-T25's fuzz half: random cluster lengths from 0 to 5,000 bytes never
    /// desync the decoder, and random bytes never panic it.
    #[test]
    fn random_rows_never_desync() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for _ in 0..200 {
            let mut row = Vec::new();
            for _ in 0..1 + rng.below(6) {
                let cells = (0..1 + rng.below(8))
                    .map(|_| {
                        let len = rng.below(5001) as usize;
                        let text = "é".repeat(len / 2) + &"x".repeat(len % 2);
                        Cell {
                            text,
                            wide: rng.below(2) == 1,
                        }
                    })
                    .collect();
                row.push(Run {
                    style: 1 + rng.below(5000) as u32,
                    link: rng.below(3) as u32,
                    cells,
                });
            }
            let back = decode(&encode(&row)).unwrap();
            assert_eq!(back.len(), row.len());
            for (a, b) in back.iter().zip(&row) {
                assert_eq!((a.style, a.link), (b.style, b.link));
                for (x, y) in a.cells.iter().zip(&b.cells) {
                    let want = if y.text.len() > MAX_CLUSTER_BYTES {
                        REPLACEMENT
                    } else {
                        &y.text
                    };
                    assert_eq!((x.text.as_str(), x.wide), (want, y.wide));
                }
            }
        }
        for _ in 0..2000 {
            let junk: Vec<u8> = (0..rng.below(64)).map(|_| rng.below(256) as u8).collect();
            let _ = decode(&junk);
        }
    }

    #[test]
    fn malformed_rows_are_errors() {
        assert_eq!(decode(&[0, 0, 4, b'a']), Err(DecodeError::TooLong));
        assert_eq!(decode(&[0, 0, 2, 0x07]), Err(DecodeError::NotAscii));
        assert_eq!(decode(&[0, 0, 3, 0x80]), Err(DecodeError::Reserved));
        assert_eq!(decode(&[0, 0, 3, 2, 0xff, 0xfe]), Err(DecodeError::NotUtf8));
        assert_eq!(decode(&[0x80; 11]), Err(DecodeError::Overlong));
        assert_eq!(decode(&[0, 0]), Err(DecodeError::Truncated));
        // A cluster longer than any the encoder writes, even when the bytes
        // are there.
        let mut big = vec![0, 0, 3, LEN_ESCAPE];
        varint(&mut big, MAX_CLUSTER_BYTES as u64 + 1);
        big.extend(std::iter::repeat_n(b'a', MAX_CLUSTER_BYTES + 1));
        assert_eq!(decode(&big), Err(DecodeError::TooLong));
    }

    /// The writer's bytes are [`encode`]'s for the same cells grouped into
    /// runs, over random rows with blanks, wide cells and long clusters.
    #[test]
    fn the_writer_writes_what_encode_writes() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut w = RowWriter::new();
        let mut out = Vec::new();
        for _ in 0..500 {
            let n = rng.below(40) as usize;
            let mut cells: Vec<(u32, u32, Cell)> = Vec::with_capacity(n);
            for _ in 0..n {
                let style = [0, 0, 0, 1, 7][rng.below(5) as usize];
                let link = [0, 0, 0, 2][rng.below(4) as usize];
                let cell = match rng.below(8) {
                    0 => Cell::default(),
                    1 => Cell::new(" "),
                    2 => Cell::wide("日"),
                    3 => Cell::new("e\u{301}"),
                    4 => Cell::new("x".repeat(rng.below(5000) as usize)),
                    _ => Cell::new(((b'a' + rng.below(26) as u8) as char).to_string()),
                };
                cells.push((style, link, cell));
            }
            let mut runs: Vec<Run> = Vec::new();
            for (style, link, cell) in &cells {
                match runs.last_mut() {
                    Some(r) if r.style == *style && r.link == *link => r.cells.push(cell.clone()),
                    _ => runs.push(Run {
                        style: *style,
                        link: *link,
                        cells: vec![cell.clone()],
                    }),
                }
            }
            out.clear();
            for (style, link, cell) in &cells {
                match cell.text.as_bytes() {
                    [b @ 0x20..=0x7e] if !cell.wide && rng.below(2) == 0 => {
                        w.ascii(&mut out, *style, *link, *b)
                    }
                    _ => w.cell(&mut out, *style, *link, &cell.text, cell.wide),
                }
            }
            w.finish(&mut out);
            assert_eq!(out, encode(&runs), "{cells:?}");
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }
}
