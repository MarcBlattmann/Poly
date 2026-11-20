//! XML property lists (Apple's public "PropertyList-1.0" DTD and the
//! `plist(5)` manual page): the reader CoreFoundation uses for a bundle's
//! `Info.plist`.

use alloc::string::String;
use alloc::vec::Vec;

/// One property list value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Real(f64),
    Bool(bool),
    /// Seconds since 2001-01-01 00:00:00 UTC (`CFAbsoluteTime`).
    Date(f64),
    Data(Vec<u8>),
    Array(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

struct Reader<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn skip_space(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn starts(&self, p: &[u8]) -> bool {
        self.s[self.i..].starts_with(p)
    }

    fn skip_until(&mut self, end: &[u8]) -> Option<()> {
        let at = find(&self.s[self.i..], end)?;
        self.i += at + end.len();
        Some(())
    }

    /// Skip white space, the XML declaration, a DOCTYPE and comments.
    fn skip_misc(&mut self) -> Option<()> {
        loop {
            self.skip_space();
            if self.starts(b"<?") {
                self.skip_until(b"?>")?;
            } else if self.starts(b"<!--") {
                self.skip_until(b"-->")?;
            } else if self.starts(b"<!") {
                self.skip_until(b">")?;
            } else {
                return Some(());
            }
        }
    }

    /// A tag at the cursor: (name, closing, empty).
    fn tag(&mut self) -> Option<(&'a [u8], bool, bool)> {
        self.skip_misc()?;
        if !self.starts(b"<") {
            return None;
        }
        self.i += 1;
        let closing = self.starts(b"/");
        if closing {
            self.i += 1;
        }
        let start = self.i;
        while self.i < self.s.len() && !matches!(self.s[self.i], b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
        let name = &self.s[start..self.i];
        let end = find(&self.s[self.i..], b">")?;
        let empty = end > 0 && self.s[self.i + end - 1] == b'/';
        self.i += end + 1;
        Some((name, closing, empty))
    }

    /// The text up to `</name>`, consumed with it.
    fn text(&mut self, name: &[u8]) -> Option<&'a [u8]> {
        let start = self.i;
        let at = find(&self.s[start..], b"</")?;
        self.i = start + at;
        let (n, closing, _) = self.tag()?;
        (closing && n == name).then_some(&self.s[start..start + at])
    }

    fn value(&mut self) -> Option<Value> {
        let (name, closing, empty) = self.tag()?;
        if closing {
            return None;
        }
        Some(match name {
            b"dict" => {
                let mut pairs = Vec::new();
                if !empty {
                    loop {
                        let save = self.i;
                        let (n, closing, empty) = self.tag()?;
                        if closing && n == b"dict" {
                            break;
                        }
                        if n != b"key" {
                            return None;
                        }
                        let key = if empty { String::new() } else { unescape(self.text(b"key")?)? };
                        let _ = save;
                        let v = self.value()?;
                        if !pairs.iter().any(|(k, _): &(String, Value)| *k == key) {
                            pairs.push((key, v));
                        }
                    }
                }
                Value::Dict(pairs)
            }
            b"array" => {
                let mut items = Vec::new();
                if !empty {
                    loop {
                        let save = self.i;
                        let (n, closing, _) = self.tag()?;
                        if closing && n == b"array" {
                            break;
                        }
                        self.i = save;
                        items.push(self.value()?);
                    }
                }
                Value::Array(items)
            }
            b"string" => Value::String(if empty { String::new() } else { unescape(self.text(b"string")?)? }),
            b"integer" => Value::Integer(parse_int(trim(self.text(b"integer")?))?),
            b"real" => Value::Real(parse_real(trim(self.text(b"real")?))?),
            b"true" => {
                if !empty {
                    self.text(b"true")?;
                }
                Value::Bool(true)
            }
            b"false" => {
                if !empty {
                    self.text(b"false")?;
                }
                Value::Bool(false)
            }
            b"date" => Value::Date(parse_date(trim(self.text(b"date")?))?),
            b"data" => Value::Data(if empty { Vec::new() } else { base64(self.text(b"data")?)? }),
            _ => return None,
        })
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(s.len());
    let b = s.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(a, |i| i + 1);
    &s[a..b]
}

/// XML text with its five entities and character references.
fn unescape(s: &[u8]) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'&' {
            let end = i + s[i..].iter().position(|&b| b == b';')?;
            let ent = &s[i + 1..end];
            let c: u32 = match ent {
                b"amp" => '&' as u32,
                b"lt" => '<' as u32,
                b"gt" => '>' as u32,
                b"quot" => '"' as u32,
                b"apos" => '\'' as u32,
                _ if ent.starts_with(b"#x") => u32::from_str_radix(core::str::from_utf8(&ent[2..]).ok()?, 16).ok()?,
                _ if ent.starts_with(b"#") => core::str::from_utf8(&ent[1..]).ok()?.parse().ok()?,
                _ => return None,
            };
            let mut buf = [0u8; 4];
            out.extend_from_slice(char::from_u32(c)?.encode_utf8(&mut buf).as_bytes());
            i = end + 1;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn parse_int(s: &[u8]) -> Option<i64> {
    let t = core::str::from_utf8(s).ok()?;
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).ok().map(|v| v as i64);
    }
    t.parse::<i64>().ok().or_else(|| t.parse::<u64>().ok().map(|v| v as i64))
}

fn parse_real(s: &[u8]) -> Option<f64> {
    let t = core::str::from_utf8(s).ok()?;
    match t {
        "nan" | "NaN" => Some(f64::NAN),
        "+infinity" | "infinity" | "inf" => Some(f64::INFINITY),
        "-infinity" | "-inf" => Some(f64::NEG_INFINITY),
        _ => t.parse().ok(),
    }
}

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds between 1970-01-01 and 2001-01-01 (`kCFAbsoluteTimeIntervalSince1970`).
pub const SECONDS_1970_TO_2001: f64 = 978_307_200.0;

/// `YYYY-MM-DDTHH:MM:SSZ` as a `CFAbsoluteTime`.
fn parse_date(s: &[u8]) -> Option<f64> {
    let t = core::str::from_utf8(s).ok()?;
    let num = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = if t.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    let unix = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec;
    Some(unix as f64 - SECONDS_1970_TO_2001)
}

/// Base64 (RFC 4648), white space ignored.
pub fn base64(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Read an XML property list.
pub fn parse(text: &[u8]) -> Option<Value> {
    let mut r = Reader { s: text, i: 0 };
    let save = r.i;
    let (name, _, empty) = r.tag()?;
    if name == b"plist" {
        if empty {
            return None;
        }
        r.value()
    } else {
        r.i = save;
        r.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
        <plist version=\"1.0\"><dict>\n\
        <key>CFBundleIdentifier</key><string>test.poly.probe</string>\n\
        <!-- a comment -->\n\
        <key>MyInt</key><integer>7</integer><key>MyBool</key><true/>\n\
        <key>MyArr</key><array><string>a</string><string>b &amp; c</string></array>\n\
        <key>Empty</key><string/><key>R</key><real>2.5</real><key>D</key><data>aGk=</data>\n\
        <key>When</key><date>2001-01-01T00:01:00Z</date><key>Inner</key><dict/>\n\
        </dict></plist>\n";

    #[test]
    fn reads_an_info_plist() {
        let Value::Dict(d) = parse(PLIST.as_bytes()).unwrap() else { panic!() };
        let get = |k: &str| d.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("CFBundleIdentifier"), Value::String("test.poly.probe".into()));
        assert_eq!(get("MyInt"), Value::Integer(7));
        assert_eq!(get("MyBool"), Value::Bool(true));
        assert_eq!(get("MyArr"), Value::Array(vec![Value::String("a".into()), Value::String("b & c".into())]));
        assert_eq!(get("Empty"), Value::String(String::new()));
        assert_eq!(get("R"), Value::Real(2.5));
        assert_eq!(get("D"), Value::Data(b"hi".to_vec()));
        assert_eq!(get("When"), Value::Date(60.0));
        assert_eq!(get("Inner"), Value::Dict(Vec::new()));
        assert_eq!(d.len(), 9);
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(parse(b"<plist><dict><key>a</key></dict></plist>").is_none());
        assert!(parse(b"not xml").is_none());
        assert_eq!(days_from_civil(2001, 1, 1) * 86400, SECONDS_1970_TO_2001 as i64);
    }
}
