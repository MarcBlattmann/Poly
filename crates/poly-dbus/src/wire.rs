//! The D-Bus type system and its marshalling (the D-Bus Specification,
//! "Type System" and "Marshaling (Wire Format)").
//!
//! Every value starts at a multiple of its alignment, counted from the
//! start of the message: 1 for `y g v`, 2 for `n q`, 4 for `b i u h s o a`,
//! 8 for `x t d ( {`. Strings and object paths are a `u32` byte length,
//! the bytes and a NUL; signatures a byte length, the bytes and a NUL; an
//! array is a `u32` byte length (not counting the padding before its first
//! element), padding to its element's alignment, then the elements; a
//! variant is a signature of one complete type and a value of it. Values
//! are written little-endian (`l`) and read in either byte order.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Why bytes are not a valid value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);

pub type Result<T> = core::result::Result<T, Error>;

/// The deepest nesting of containers the specification allows (32 arrays
/// plus 32 structures), and of variants inside variants (ours).
const MAX_DEPTH: usize = 64;
/// The longest signature.
pub const MAX_SIGNATURE: usize = 255;
/// The longest array, in bytes.
pub const MAX_ARRAY: usize = 64 << 20;

/// A D-Bus value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    Int16(i16),
    Uint16(u16),
    Int32(i32),
    Uint32(u32),
    Int64(i64),
    Uint64(u64),
    Double(f64),
    Str(String),
    Path(String),
    Signature(String),
    /// The element type's signature and the elements (dict entries for a
    /// dictionary: element signature `{sv}` and so on).
    Array(String, Vec<Value>),
    Struct(Vec<Value>),
    DictEntry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
    /// A Unix file descriptor: its index into the message's descriptors.
    Fd(u32),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }

    /// An array of strings (`as`).
    pub fn strings(items: &[&str]) -> Value {
        Value::Array("s".into(), items.iter().map(|s| Value::str(s)).collect())
    }

    /// The value's signature (one complete type).
    pub fn signature(&self) -> String {
        let code = |c: char| c.to_string();
        match self {
            Value::Byte(_) => code('y'),
            Value::Bool(_) => code('b'),
            Value::Int16(_) => code('n'),
            Value::Uint16(_) => code('q'),
            Value::Int32(_) => code('i'),
            Value::Uint32(_) => code('u'),
            Value::Int64(_) => code('x'),
            Value::Uint64(_) => code('t'),
            Value::Double(_) => code('d'),
            Value::Str(_) => code('s'),
            Value::Path(_) => code('o'),
            Value::Signature(_) => code('g'),
            Value::Array(elem, _) => alloc::format!("a{elem}"),
            Value::Struct(fields) => {
                let mut s = String::from("(");
                for f in fields {
                    s.push_str(&f.signature());
                }
                s.push(')');
                s
            }
            Value::DictEntry(k, v) => alloc::format!("{{{}{}}}", k.signature(), v.signature()),
            Value::Variant(_) => code('v'),
            Value::Fd(_) => code('h'),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Path(s) | Value::Signature(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Value::Uint32(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Value::Int32(v) => Some(*v),
            _ => None,
        }
    }

    /// An integer of any width, widened (hints such as `urgency` come as a
    /// byte but some senders use other widths).
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Byte(v) => Some(v.into()),
            Value::Int16(v) => Some(v.into()),
            Value::Uint16(v) => Some(v.into()),
            Value::Int32(v) => Some(v.into()),
            Value::Uint32(v) => Some(v.into()),
            Value::Int64(v) => Some(v),
            Value::Uint64(v) => i64::try_from(v).ok(),
            Value::Bool(v) => Some(v.into()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(_, items) => Some(items),
            _ => None,
        }
    }

    /// What a variant holds (itself for anything else).
    pub fn unvariant(&self) -> &Value {
        match self {
            Value::Variant(v) => v.unvariant(),
            v => v,
        }
    }
}

/// The alignment of values whose signature starts with `code`.
pub fn alignment(code: u8) -> usize {
    match code {
        b'n' | b'q' => 2,
        b'b' | b'i' | b'u' | b'h' | b's' | b'o' | b'a' => 4,
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        _ => 1,
    }
}

fn is_basic(code: u8) -> bool {
    matches!(code, b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b's' | b'o' | b'g' | b'h')
}

/// The length of the first complete type in `sig`.
pub fn complete_type(sig: &[u8]) -> Result<usize> {
    fn one(sig: &[u8], depth: usize) -> Result<usize> {
        if depth > MAX_DEPTH {
            return Err(Error("signature nested too deeply"));
        }
        match sig.first() {
            None => Err(Error("signature ends early")),
            Some(&c) if is_basic(c) || c == b'v' => Ok(1),
            Some(b'a') => {
                if sig.get(1) == Some(&b'{') {
                    // A dict entry: a basic key and one complete value.
                    let key = *sig.get(2).ok_or(Error("dict entry ends early"))?;
                    if !is_basic(key) {
                        return Err(Error("a dict entry's key must be a basic type"));
                    }
                    let v = one(&sig[3..], depth + 1)?;
                    if sig.get(3 + v) != Some(&b'}') {
                        return Err(Error("a dict entry holds a key and one value"));
                    }
                    Ok(4 + v)
                } else {
                    Ok(1 + one(&sig[1..], depth + 1)?)
                }
            }
            Some(b'(') => {
                let mut i = 1;
                while sig.get(i) != Some(&b')') {
                    if i >= sig.len() {
                        return Err(Error("structure not closed"));
                    }
                    i += one(&sig[i..], depth + 1)?;
                }
                if i == 1 {
                    return Err(Error("empty structure"));
                }
                Ok(i + 1)
            }
            Some(_) => Err(Error("unknown type code")),
        }
    }
    one(sig, 0)
}

/// Whether `sig` is a valid signature (any number of complete types).
pub fn valid_signature(sig: &str) -> bool {
    let b = sig.as_bytes();
    if b.len() > MAX_SIGNATURE {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        match complete_type(&b[i..]) {
            Ok(n) => i += n,
            Err(_) => return false,
        }
    }
    true
}

/// Split a signature into its complete types.
pub fn split_signature(sig: &str) -> Result<Vec<&str>> {
    let b = sig.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let n = complete_type(&b[i..])?;
        out.push(&sig[i..i + n]);
        i += n;
    }
    Ok(out)
}

/// Whether `p` is a valid object path: `/`, or `/` separated elements of
/// `[A-Za-z0-9_]`, none empty, no trailing `/`.
pub fn valid_path(p: &str) -> bool {
    if p == "/" {
        return true;
    }
    let Some(rest) = p.strip_prefix('/') else { return false };
    rest.split('/').all(|e| !e.is_empty() && e.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
}

/// Writes values, little-endian, aligned from the start of the buffer.
#[derive(Debug, Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    pub fn pad(&mut self, align: usize) {
        while !self.buf.len().is_multiple_of(align) {
            self.buf.push(0);
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.pad(4);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    fn signature(&mut self, s: &str) {
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    pub fn value(&mut self, v: &Value) {
        match v {
            Value::Byte(b) => self.buf.push(*b),
            Value::Bool(b) => self.u32(u32::from(*b)),
            Value::Int16(x) => {
                self.pad(2);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Uint16(x) => {
                self.pad(2);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Int32(x) => {
                self.pad(4);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Uint32(x) | Value::Fd(x) => self.u32(*x),
            Value::Int64(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Uint64(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_le_bytes());
            }
            Value::Double(x) => {
                self.pad(8);
                self.buf.extend_from_slice(&x.to_bits().to_le_bytes());
            }
            Value::Str(s) | Value::Path(s) => self.string(s),
            Value::Signature(s) => self.signature(s),
            Value::Array(elem, items) => {
                self.u32(0);
                let at = self.buf.len() - 4;
                self.pad(alignment(elem.as_bytes().first().copied().unwrap_or(b'y')));
                let start = self.buf.len();
                for item in items {
                    self.value(item);
                }
                let len = (self.buf.len() - start) as u32;
                self.buf[at..at + 4].copy_from_slice(&len.to_le_bytes());
            }
            Value::Struct(fields) => {
                self.pad(8);
                for f in fields {
                    self.value(f);
                }
            }
            Value::DictEntry(k, v) => {
                self.pad(8);
                self.value(k);
                self.value(v);
            }
            Value::Variant(inner) => {
                self.signature(&inner.signature());
                self.value(inner);
            }
        }
    }
}

/// Reads values in either byte order, aligned from the start of `buf`.
#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
    big_endian: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big_endian: bool) -> Reader<'a> {
        Reader { buf, pos: 0, big_endian }
    }

    pub fn at(buf: &'a [u8], pos: usize, big_endian: bool) -> Reader<'a> {
        Reader { buf, pos, big_endian }
    }

    fn align(&mut self, a: usize) -> Result<()> {
        let to = self.pos.next_multiple_of(a);
        if to > self.buf.len() {
            return Err(Error("padding past the end"));
        }
        if self.buf[self.pos..to].iter().any(|&b| b != 0) {
            return Err(Error("padding is not zero"));
        }
        self.pos = to;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error("length overflows"))?;
        let b = self.buf.get(self.pos..end).ok_or(Error("value past the end"))?;
        self.pos = end;
        Ok(b)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.align(N)?;
        let mut b: [u8; N] = self.take(N)?.try_into().map_err(|_| Error("short"))?;
        if self.big_endian {
            b.reverse();
        }
        Ok(b)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    fn text(bytes: &[u8]) -> Result<String> {
        if bytes.contains(&0) {
            return Err(Error("a NUL inside a string"));
        }
        core::str::from_utf8(bytes).map(String::from).map_err(|_| Error("a string is not UTF-8"))
    }

    fn string(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let s = Self::text(self.take(n)?)?;
        if self.take(1)? != [0] {
            return Err(Error("a string without its NUL"));
        }
        Ok(s)
    }

    pub fn signature(&mut self) -> Result<String> {
        let n = usize::from(self.take(1)?[0]);
        let s = Self::text(self.take(n)?)?;
        if self.take(1)? != [0] {
            return Err(Error("a signature without its NUL"));
        }
        if !valid_signature(&s) {
            return Err(Error("not a valid signature"));
        }
        Ok(s)
    }

    /// One value of the complete type `sig`.
    pub fn value(&mut self, sig: &str) -> Result<Value> {
        self.value_at(sig.as_bytes(), 0)
    }

    fn value_at(&mut self, sig: &[u8], depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(Error("values nested too deeply"));
        }
        let code = *sig.first().ok_or(Error("empty signature"))?;
        Ok(match code {
            b'y' => Value::Byte(self.take(1)?[0]),
            b'b' => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Error("a boolean is neither 0 nor 1")),
            },
            b'n' => Value::Int16(i16::from_le_bytes(self.fixed()?)),
            b'q' => Value::Uint16(u16::from_le_bytes(self.fixed()?)),
            b'i' => Value::Int32(i32::from_le_bytes(self.fixed()?)),
            b'u' => Value::Uint32(self.u32()?),
            b'h' => Value::Fd(self.u32()?),
            b'x' => Value::Int64(i64::from_le_bytes(self.fixed()?)),
            b't' => Value::Uint64(u64::from_le_bytes(self.fixed()?)),
            b'd' => Value::Double(f64::from_bits(u64::from_le_bytes(self.fixed()?))),
            b's' => Value::Str(self.string()?),
            b'o' => {
                let p = self.string()?;
                if !valid_path(&p) {
                    return Err(Error("not a valid object path"));
                }
                Value::Path(p)
            }
            b'g' => Value::Signature(self.signature()?),
            b'v' => {
                let s = self.signature()?;
                if s.is_empty() || complete_type(s.as_bytes())? != s.len() {
                    return Err(Error("a variant holds one complete type"));
                }
                Value::Variant(Box::new(self.value_at(s.as_bytes(), depth + 1)?))
            }
            b'a' => {
                let len = self.u32()? as usize;
                if len > MAX_ARRAY {
                    return Err(Error("array too long"));
                }
                let elem_len = complete_type(sig)? - 1;
                let elem = &sig[1..1 + elem_len];
                self.align(alignment(elem[0]))?;
                let end = self.pos.checked_add(len).ok_or(Error("length overflows"))?;
                if end > self.buf.len() {
                    return Err(Error("array past the end"));
                }
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.value_at(elem, depth + 1)?);
                }
                if self.pos != end {
                    return Err(Error("array elements overrun its length"));
                }
                let elem = core::str::from_utf8(elem).map_err(|_| Error("signature"))?;
                Value::Array(elem.into(), items)
            }
            b'(' => {
                self.align(8)?;
                let n = complete_type(sig)?;
                let mut fields = Vec::new();
                let mut i = 1;
                while i < n - 1 {
                    let f = complete_type(&sig[i..])?;
                    fields.push(self.value_at(&sig[i..i + f], depth + 1)?);
                    i += f;
                }
                Value::Struct(fields)
            }
            b'{' => {
                self.align(8)?;
                let k = self.value_at(&sig[1..2], depth + 1)?;
                let vl = complete_type(&sig[2..])?;
                let v = self.value_at(&sig[2..2 + vl], depth + 1)?;
                Value::DictEntry(Box::new(k), Box::new(v))
            }
            _ => return Err(Error("unknown type code")),
        })
    }
}

/// `values` marshalled from offset 0 (a body starts 8-aligned).
pub fn marshal(values: &[Value]) -> Vec<u8> {
    let mut w = Writer::new();
    for v in values {
        w.value(v);
    }
    w.buf
}

/// The values of signature `sig` in `body`, which must hold exactly them.
pub fn unmarshal(body: &[u8], sig: &str, big_endian: bool) -> Result<Vec<Value>> {
    let mut r = Reader::new(body, big_endian);
    let mut out = Vec::new();
    for t in split_signature(sig)? {
        out.push(r.value(t)?);
    }
    if r.pos != body.len() {
        return Err(Error("bytes after the body's values"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn signatures() {
        assert!(valid_signature(""));
        assert!(valid_signature("susssasa{sv}i"));
        assert!(valid_signature("a(ii)v"));
        assert!(!valid_signature("a"));
        assert!(!valid_signature("()"));
        assert!(!valid_signature("{sv}"), "a dict entry only inside an array");
        assert!(!valid_signature("a{vs}"), "the key is basic");
        assert!(!valid_signature("(ii"));
        assert!(!valid_signature("z"));
        assert_eq!(split_signature("susssasa{sv}i").unwrap(), ["s", "u", "s", "s", "s", "as", "a{sv}", "i"]);
        assert_eq!(complete_type(b"a{sa(ii)}u").unwrap(), 9);
    }

    #[test]
    fn paths() {
        assert!(valid_path("/"));
        assert!(valid_path("/org/freedesktop/Notifications"));
        assert!(!valid_path("/org/"));
        assert!(!valid_path("org"));
        assert!(!valid_path("/a//b"));
        assert!(!valid_path("/a-b"));
    }

    #[test]
    fn marshalling_by_the_specification() {
        // A byte, then a u32 aligned to 4, then a string.
        let b = marshal(&[Value::Byte(7), Value::Uint32(0x0102_0304), Value::str("hi")]);
        assert_eq!(b, [7, 0, 0, 0, 4, 3, 2, 1, 2, 0, 0, 0, b'h', b'i', 0]);
        // An array of i64: the length counts no padding before the first
        // element, which sits at 8.
        let b = marshal(&[Value::Array("x".into(), vec![Value::Int64(-1)])]);
        assert_eq!(b, [8, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        // An empty array of structures still pads to 8.
        let b = marshal(&[Value::Byte(1), Value::Array("(ii)".into(), vec![])]);
        assert_eq!(b, [1, 0, 0, 0, 0, 0, 0, 0]);
        // A variant: its signature, then the value.
        let b = marshal(&[Value::Variant(Box::new(Value::Byte(2)))]);
        assert_eq!(b, [1, b'y', 0, 2]);
    }

    #[test]
    fn round_trips_and_byte_orders() {
        let hints = Value::Array(
            "{sv}".into(),
            vec![
                Value::DictEntry(Box::new(Value::str("urgency")), Box::new(Value::Variant(Box::new(Value::Byte(2))))),
                Value::DictEntry(
                    Box::new(Value::str("x")),
                    Box::new(Value::Variant(Box::new(Value::Struct(vec![Value::Int32(-5), Value::Double(1.5)])))),
                ),
            ],
        );
        let values = vec![
            Value::str("notify-send"),
            Value::Uint32(0),
            Value::str(""),
            Value::str("Hello"),
            Value::str("World"),
            Value::strings(&["default", "Open"]),
            hints,
            Value::Int32(-1),
            Value::Path("/a/b".into()),
            Value::Signature("a{sv}".into()),
            Value::Bool(true),
            Value::Int16(-2),
            Value::Uint64(1 << 40),
        ];
        let sig: String = values.iter().map(Value::signature).collect();
        assert_eq!(sig, "susssasa{sv}iogbnt");
        let bytes = marshal(&values);
        assert_eq!(unmarshal(&bytes, &sig, false).unwrap(), values);
        // Big-endian: a u32 and a string.
        let be = [0, 0, 0, 5, 0, 0, 0, 2, b'o', b'k', 0];
        assert_eq!(unmarshal(&be, "us", true).unwrap(), [Value::Uint32(5), Value::str("ok")]);
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(unmarshal(&[2, 0, 0, 0], "b", false).is_err(), "a boolean of 2");
        assert!(unmarshal(&[1, 0, 0, 0, b'a', 1], "s", false).is_err(), "no NUL");
        assert!(unmarshal(&[1, 0, 0, 0, 0, 0], "s", false).is_err(), "a NUL inside");
        assert!(unmarshal(&[0xff, 0xff, 0, 0], "ay", false).is_err(), "past the end");
        assert!(unmarshal(&[7, 1, 0, 0, 0, 0, 0, 0], "yu", false).is_err(), "padding not zero");
        assert!(unmarshal(&[2, b'i', b'i', 0], "v", false).is_err(), "a variant of two types");
        assert!(unmarshal(&[1, 0, 0, 0, 5], "u", false).is_err(), "extra bytes");
        assert!(unmarshal(&[2, 0, 0, 0, b'/', b'/', 0], "o", false).is_err(), "a bad path");
    }
}
