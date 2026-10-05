//! The subset of CBOR (RFC 8949) the grid protocol's payloads use: unsigned
//! and negative integers, byte and text strings, arrays, maps, booleans,
//! null and floats (Terminal State Protocol §7).
//!
//! Written here rather than taken from a crate because the protocol needs
//! very little of CBOR and both ends must agree on exactly that little: maps
//! keyed by small integers, so a field can be added without breaking an
//! older reader, and row cells as byte strings, so the hot path stays
//! compact.
//!
//! The writer writes maps of unknown size as indefinite-length maps, so a
//! struct is written field by field with no count up front. The reader takes
//! both forms. It never panics and never allocates more than its input could
//! hold: every length is checked against the bytes left before anything is
//! reserved, and nesting is bounded.

use std::fmt;

/// How deep arrays and maps may nest. The protocol needs four.
const MAX_DEPTH: usize = 16;

/// Why a payload could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CborError {
    /// The bytes end inside an item.
    Truncated,
    /// Something this subset does not use: tags, simple values, reserved
    /// additional information, a break outside an indefinite item.
    Unsupported(u8),
    /// A length larger than the bytes that could hold it.
    TooLong,
    /// Nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A text string that is not UTF-8.
    NotUtf8,
    /// Bytes after the top-level item.
    Trailing,
}

impl fmt::Display for CborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CborError::Truncated => write!(f, "CBOR ends inside an item"),
            CborError::Unsupported(b) => write!(f, "CBOR item 0x{b:02x} is not supported"),
            CborError::TooLong => write!(f, "CBOR length exceeds the payload"),
            CborError::TooDeep => write!(f, "CBOR nests deeper than {MAX_DEPTH}"),
            CborError::NotUtf8 => write!(f, "CBOR text is not UTF-8"),
            CborError::Trailing => write!(f, "bytes after the CBOR item"),
        }
    }
}

impl std::error::Error for CborError {}

/// A decoded item.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Uint(u64),
    /// A negative integer, `-1 - n`.
    Neg(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
    Float(f64),
}

impl Value {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Uint(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Uint(v) => i64::try_from(*v).ok(),
            Value::Neg(n) => i64::try_from(*n).ok().map(|n| -1 - n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Uint(v) => Some(*v as f64),
            Value::Neg(n) => Some(-1.0 - *n as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn into_text(self) -> Option<String> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn into_array(self) -> Option<Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The map's fields keyed by small integers; other keys are dropped, as
    /// an older reader drops fields it does not know.
    pub fn into_fields(self) -> Option<Fields> {
        match self {
            Value::Map(entries) => Some(Fields(
                entries
                    .into_iter()
                    .filter_map(|(k, v)| Some((k.as_u64()?, v)))
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// A map's fields by integer key. Unknown keys are kept and ignored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fields(Vec<(u64, Value)>);

impl Fields {
    /// Takes field `key` out, the first one if it was sent twice.
    pub fn take(&mut self, key: u64) -> Option<Value> {
        let at = self.0.iter().position(|(k, _)| *k == key)?;
        Some(self.0.swap_remove(at).1)
    }

    pub fn u64(&mut self, key: u64) -> Option<u64> {
        self.take(key)?.as_u64()
    }

    pub fn bool(&mut self, key: u64) -> Option<bool> {
        self.take(key)?.as_bool()
    }

    pub fn text(&mut self, key: u64) -> Option<String> {
        self.take(key)?.into_text()
    }

    pub fn bytes(&mut self, key: u64) -> Option<Vec<u8>> {
        self.take(key)?.into_bytes()
    }

    pub fn array(&mut self, key: u64) -> Option<Vec<Value>> {
        self.take(key)?.into_array()
    }

    pub fn fields(&mut self, key: u64) -> Option<Fields> {
        self.take(key)?.into_fields()
    }
}

/// Reads one item that fills `bytes` exactly.
pub fn decode(bytes: &[u8]) -> Result<Value, CborError> {
    let mut r = Reader { bytes, at: 0 };
    let v = r.item(0)?;
    if r.at != bytes.len() {
        return Err(CborError::Trailing);
    }
    Ok(v)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// What a head byte and its argument say.
enum Head {
    Item(u8, u64),
    /// Major type with additional information 31: indefinite length, or a
    /// break for major 7.
    Indefinite(u8),
}

impl Reader<'_> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize) -> Result<&[u8], CborError> {
        if n > self.left() {
            return Err(CborError::Truncated);
        }
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn head(&mut self) -> Result<(u8, Head), CborError> {
        let b = *self.take(1)?.first().ok_or(CborError::Truncated)?;
        let major = b >> 5;
        let info = b & 0x1f;
        let arg = match info {
            0..=23 => u64::from(info),
            24 => u64::from(self.take(1)?[0]),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?.try_into().map_err(|_| CborError::Truncated)?,
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?.try_into().map_err(|_| CborError::Truncated)?,
            )),
            27 => u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| CborError::Truncated)?),
            31 => return Ok((b, Head::Indefinite(major))),
            _ => return Err(CborError::Unsupported(b)),
        };
        Ok((b, Head::Item(major, arg)))
    }

    /// A length that must fit in the bytes left, one byte an element at
    /// least.
    fn len(&self, n: u64) -> Result<usize, CborError> {
        usize::try_from(n)
            .ok()
            .filter(|&n| n <= self.left())
            .ok_or(CborError::TooLong)
    }

    fn is_break(&self) -> bool {
        self.bytes.get(self.at) == Some(&0xff)
    }

    fn item(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError::TooDeep);
        }
        let (b, head) = self.head()?;
        match head {
            Head::Item(0, v) => Ok(Value::Uint(v)),
            Head::Item(1, v) => Ok(Value::Neg(v)),
            Head::Item(2, n) => {
                let n = self.len(n)?;
                Ok(Value::Bytes(self.take(n)?.to_vec()))
            }
            Head::Item(3, n) => {
                let n = self.len(n)?;
                let s = std::str::from_utf8(self.take(n)?).map_err(|_| CborError::NotUtf8)?;
                Ok(Value::Text(s.to_owned()))
            }
            Head::Item(4, n) => {
                let n = self.len(n)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(items))
            }
            Head::Item(5, n) => {
                // Two items an entry, so at most half the bytes left.
                let n = self.len(n.saturating_mul(2))? / 2;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    let k = self.item(depth + 1)?;
                    let v = self.item(depth + 1)?;
                    entries.push((k, v));
                }
                Ok(Value::Map(entries))
            }
            Head::Item(7, v) => match b & 0x1f {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 | 23 => Ok(Value::Null),
                25 => Ok(Value::Float(f16_to_f64(v as u16))),
                26 => Ok(Value::Float(f64::from(f32::from_bits(v as u32)))),
                27 => Ok(Value::Float(f64::from_bits(v))),
                _ => Err(CborError::Unsupported(b)),
            },
            Head::Indefinite(2) | Head::Indefinite(3) => {
                let text = b >> 5 == 3;
                let mut buf = Vec::new();
                loop {
                    if self.is_break() {
                        self.at += 1;
                        break;
                    }
                    match self.item(depth + 1)? {
                        Value::Bytes(c) if !text => buf.extend_from_slice(&c),
                        Value::Text(c) if text => buf.extend_from_slice(c.as_bytes()),
                        _ => return Err(CborError::Unsupported(b)),
                    }
                }
                if text {
                    String::from_utf8(buf)
                        .map(Value::Text)
                        .map_err(|_| CborError::NotUtf8)
                } else {
                    Ok(Value::Bytes(buf))
                }
            }
            Head::Indefinite(4) => {
                let mut items = Vec::new();
                loop {
                    if self.is_break() {
                        self.at += 1;
                        return Ok(Value::Array(items));
                    }
                    items.push(self.item(depth + 1)?);
                }
            }
            Head::Indefinite(5) => {
                let mut entries = Vec::new();
                loop {
                    if self.is_break() {
                        self.at += 1;
                        return Ok(Value::Map(entries));
                    }
                    let k = self.item(depth + 1)?;
                    let v = self.item(depth + 1)?;
                    entries.push((k, v));
                }
            }
            _ => Err(CborError::Unsupported(b)),
        }
    }
}

fn f16_to_f64(h: u16) -> f64 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f64::from(h & 0x3ff);
    sign * match exp {
        0 => frac * 2f64.powi(-24),
        31 if frac == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + frac / 1024.0) * 2f64.powi(exp - 15),
    }
}

/// Writes CBOR into a buffer the caller owns.
#[derive(Debug)]
pub struct Writer<'a> {
    pub out: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Writer { out }
    }

    fn head(&mut self, major: u8, v: u64) {
        let m = major << 5;
        if v < 24 {
            self.out.push(m | v as u8);
        } else if let Ok(v) = u8::try_from(v) {
            self.out.extend_from_slice(&[m | 24, v]);
        } else if let Ok(v) = u16::try_from(v) {
            self.out.push(m | 25);
            self.out.extend_from_slice(&v.to_be_bytes());
        } else if let Ok(v) = u32::try_from(v) {
            self.out.push(m | 26);
            self.out.extend_from_slice(&v.to_be_bytes());
        } else {
            self.out.push(m | 27);
            self.out.extend_from_slice(&v.to_be_bytes());
        }
    }

    pub fn uint(&mut self, v: u64) {
        self.head(0, v);
    }

    pub fn int(&mut self, v: i64) {
        if v >= 0 {
            self.head(0, v as u64);
        } else {
            self.head(1, (-1 - v) as u64);
        }
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.head(2, b.len() as u64);
        self.out.extend_from_slice(b);
    }

    pub fn text(&mut self, s: &str) {
        self.head(3, s.len() as u64);
        self.out.extend_from_slice(s.as_bytes());
    }

    pub fn array(&mut self, n: usize) {
        self.head(4, n as u64);
    }

    pub fn bool(&mut self, b: bool) {
        self.out.push(if b { 0xf5 } else { 0xf4 });
    }

    pub fn null(&mut self) {
        self.out.push(0xf6);
    }

    pub fn f32(&mut self, v: f32) {
        self.out.push(0xfa);
        self.out.extend_from_slice(&v.to_bits().to_be_bytes());
    }

    /// Starts a map of unknown size; [`Writer::end`] closes it.
    pub fn map(&mut self) {
        self.out.push(0xbf);
    }

    pub fn end(&mut self) {
        self.out.push(0xff);
    }

    /// Field `key` of the open map. The value is written next.
    pub fn key(&mut self, key: u64) -> &mut Self {
        self.uint(key);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_strings_and_maps_round_trip() {
        let mut out = Vec::new();
        let mut w = Writer::new(&mut out);
        w.map();
        for (k, v) in [0u64, 23, 24, 255, 256, 65_535, 65_536, u64::MAX]
            .iter()
            .enumerate()
        {
            w.key(k as u64).uint(*v);
        }
        w.key(10).int(-1);
        w.key(11).int(i64::MIN);
        w.key(12).text("héllo");
        w.key(13).bytes(&[0, 1, 2]);
        w.key(14).bool(true);
        w.key(15).null();
        w.key(16).f32(1.5);
        w.key(17).array(2);
        w.uint(1);
        w.text("x");
        w.end();
        let mut f = decode(&out).unwrap().into_fields().unwrap();
        assert_eq!(f.u64(7), Some(u64::MAX));
        assert_eq!(f.u64(2), Some(24));
        assert_eq!(f.take(10).unwrap().as_i64(), Some(-1));
        assert_eq!(f.take(11).unwrap().as_i64(), Some(i64::MIN));
        assert_eq!(f.text(12).as_deref(), Some("héllo"));
        assert_eq!(f.bytes(13), Some(vec![0, 1, 2]));
        assert_eq!(f.bool(14), Some(true));
        assert_eq!(f.take(15), Some(Value::Null));
        assert_eq!(f.take(16).unwrap().as_f64(), Some(1.5));
        assert_eq!(f.array(17).unwrap().len(), 2);
        assert_eq!(f.u64(99), None);
    }

    #[test]
    fn definite_maps_and_indefinite_strings_are_read() {
        // {1: "ab"} with a definite map and a chunked text string.
        let bytes = [0xa1, 0x01, 0x7f, 0x61, b'a', 0x61, b'b', 0xff];
        let mut f = decode(&bytes).unwrap().into_fields().unwrap();
        assert_eq!(f.text(1).as_deref(), Some("ab"));
        // Half-precision 1.0.
        assert_eq!(decode(&[0xf9, 0x3c, 0x00]).unwrap(), Value::Float(1.0));
    }

    #[test]
    fn hostile_input_is_an_error_never_a_panic() {
        assert_eq!(decode(&[]), Err(CborError::Truncated));
        assert_eq!(
            decode(&[0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
            Err(CborError::TooLong)
        );
        assert_eq!(
            decode(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
            Err(CborError::TooLong)
        );
        assert_eq!(decode(&[0xc0, 0x00]), Err(CborError::Unsupported(0xc0)));
        assert_eq!(decode(&[0x01, 0x02]), Err(CborError::Trailing));
        assert_eq!(decode(&[0x62, 0xff, 0xfe]), Err(CborError::NotUtf8));
        assert_eq!(decode(&[0x9f; 64]), Err(CborError::TooDeep));
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = (seed % 24) as usize;
            let junk: Vec<u8> = (0..n).map(|i| (seed >> (i % 8 * 8)) as u8).collect();
            let _ = decode(&junk);
        }
    }
}
