//! A small XML reader and writer: what task documents use (elements, attributes, text, the five
//! predefined entities and character references, comments, the XML declaration, CDATA).
//!
//! The writer makes the Task Scheduler's text form (TASKSCHD.md section 4): CR LF line ends, two
//! spaces of indentation per level, `<Name />` for an empty element.

use alloc::string::String;
use alloc::vec::Vec;

/// An element: its name, attributes, child elements, and the text directly inside it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    pub text: String,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// A parsed document: the declaration's `encoding` (if any) and the root element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub encoding: Option<String>,
    pub root: Element,
}

/// Parse `s`; `None` when it is not well-formed XML.
pub fn parse(s: &str) -> Option<Document> {
    let mut p = Parser { s: s.as_bytes(), at: 0, src: s };
    if p.s.starts_with("\u{feff}".as_bytes()) {
        p.at = 3;
    }
    let mut encoding = None;
    p.skip_ws();
    if p.starts("<?xml") {
        let end = p.find("?>")?;
        let decl = &s[p.at + 5..end];
        encoding = decl_attr(decl, "encoding");
        p.at = end + 2;
    }
    p.misc()?;
    let root = p.element()?;
    p.misc()?;
    if p.at != p.s.len() {
        return None;
    }
    Some(Document { encoding, root })
}

fn decl_attr(decl: &str, name: &str) -> Option<String> {
    let i = decl.find(name)?;
    let rest = decl[i + name.len()..].trim_start().strip_prefix('=')?.trim_start();
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let end = rest.find(q)?;
    Some(String::from(&rest[..end]))
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
    src: &'a str,
}

impl Parser<'_> {
    fn starts(&self, t: &str) -> bool {
        self.s[self.at..].starts_with(t.as_bytes())
    }

    fn find(&self, t: &str) -> Option<usize> {
        self.src[self.at..].find(t).map(|i| i + self.at)
    }

    fn skip_ws(&mut self) {
        while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b'\t' | b'\r' | b'\n') {
            self.at += 1;
        }
    }

    /// Whitespace, comments and processing instructions between top-level parts.
    fn misc(&mut self) -> Option<()> {
        loop {
            self.skip_ws();
            if self.starts("<!--") {
                self.at = self.find("-->")? + 3;
            } else if self.starts("<?") {
                self.at = self.find("?>")? + 2;
            } else {
                return Some(());
            }
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.at;
        while self.at < self.s.len() {
            let c = self.s[self.at];
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.') || c >= 0x80 {
                self.at += 1;
            } else {
                break;
            }
        }
        if self.at == start || self.s[start].is_ascii_digit() || matches!(self.s[start], b'-' | b'.') {
            return None;
        }
        Some(String::from(&self.src[start..self.at]))
    }

    fn element(&mut self) -> Option<Element> {
        if !self.starts("<") {
            return None;
        }
        self.at += 1;
        let name = self.name()?;
        let mut e = Element { name, ..Element::default() };
        loop {
            let before = self.at;
            self.skip_ws();
            if self.starts("/>") {
                self.at += 2;
                return Some(e);
            }
            if self.starts(">") {
                self.at += 1;
                break;
            }
            if self.at == before {
                return None;
            }
            let an = self.name()?;
            self.skip_ws();
            if !self.starts("=") {
                return None;
            }
            self.at += 1;
            self.skip_ws();
            let q = *self.s.get(self.at)?;
            if q != b'"' && q != b'\'' {
                return None;
            }
            self.at += 1;
            let start = self.at;
            while self.at < self.s.len() && self.s[self.at] != q {
                if self.s[self.at] == b'<' {
                    return None;
                }
                self.at += 1;
            }
            if self.at >= self.s.len() {
                return None;
            }
            let v = unescape(&self.src[start..self.at])?;
            self.at += 1;
            if e.attrs.iter().any(|(n, _)| *n == an) {
                return None;
            }
            e.attrs.push((an, v));
        }
        // Content.
        loop {
            if self.at >= self.s.len() {
                return None;
            }
            if self.starts("</") {
                self.at += 2;
                let n = self.name()?;
                self.skip_ws();
                if n != e.name || !self.starts(">") {
                    return None;
                }
                self.at += 1;
                return Some(e);
            }
            if self.starts("<!--") {
                self.at = self.find("-->")? + 3;
                continue;
            }
            if self.starts("<![CDATA[") {
                let end = self.find("]]>")?;
                e.text.push_str(&self.src[self.at + 9..end]);
                self.at = end + 3;
                continue;
            }
            if self.starts("<?") {
                self.at = self.find("?>")? + 2;
                continue;
            }
            if self.starts("<") {
                let c = self.element()?;
                e.children.push(c);
                continue;
            }
            let start = self.at;
            while self.at < self.s.len() && self.s[self.at] != b'<' {
                self.at += 1;
            }
            e.text.push_str(&unescape(&self.src[start..self.at])?);
        }
    }
}

/// Replace the entity and character references of `s`; `None` for an unknown or broken one.
pub fn unescape(s: &str) -> Option<String> {
    if !s.contains('&') {
        return Some(String::from(s));
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let end = tail.find(';')?;
        let ent = &tail[..end];
        match ent {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    u32::from_str_radix(h, 16).ok()?
                } else {
                    let d = ent.strip_prefix('#')?;
                    d.parse::<u32>().ok()?
                };
                out.push(char::from_u32(n)?);
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Text as element content: `&`, `<` and `>` escaped (quotes stay as they are).
pub fn escape_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            _ => o.push(c),
        }
    }
    o
}

/// Text as an attribute value in double quotes.
pub fn escape_attr(s: &str) -> String {
    let mut o = escape_text(s);
    if o.contains('"') {
        o = o.replace('"', "&quot;");
    }
    o
}

/// The writer: one element per line, two spaces per level, CR LF.
#[derive(Debug)]
pub struct Writer {
    pub out: String,
    depth: usize,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: String::new(), depth: 0 }
    }

    fn line(&mut self, s: &str) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
        self.out.push_str(s);
        self.out.push_str("\r\n");
    }

    fn tag(name: &str, attrs: &[(&str, &str)]) -> String {
        let mut t = String::from("<");
        t.push_str(name);
        for (n, v) in attrs {
            t.push(' ');
            t.push_str(n);
            t.push_str("=\"");
            t.push_str(&escape_attr(v));
            t.push('"');
        }
        t
    }

    /// A raw line (the XML declaration).
    pub fn raw(&mut self, s: &str) {
        self.line(s);
    }

    pub fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        let mut t = Self::tag(name, attrs);
        t.push('>');
        self.line(&t);
        self.depth += 1;
    }

    pub fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        let mut t = String::from("</");
        t.push_str(name);
        t.push('>');
        self.line(&t);
    }

    /// `<name attrs />`.
    pub fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        let mut t = Self::tag(name, attrs);
        t.push_str(" />");
        self.line(&t);
    }

    /// `<name>text</name>`.
    pub fn leaf(&mut self, name: &str, text: &str) {
        let mut t = Self::tag(name, &[]);
        t.push('>');
        t.push_str(&escape_text(text));
        t.push_str("</");
        t.push_str(name);
        t.push('>');
        self.line(&t);
    }

    /// The text without the last line end.
    pub fn finish(mut self) -> String {
        if self.out.ends_with("\r\n") {
            self.out.truncate(self.out.len() - 2);
        }
        self.out
    }
}
