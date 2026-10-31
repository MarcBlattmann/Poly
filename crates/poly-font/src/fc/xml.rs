//! Just enough XML (W3C XML 1.0) for `fonts.conf`: elements, attributes,
//! text with the predefined and numeric character references, CDATA;
//! the XML declaration, comments, processing instructions and the
//! document type declaration are skipped. No namespaces, no validation.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// An element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

/// A child of an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == name).map(|a| a.1.as_str())
    }

    /// The child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| if let Node::Element(e) = n { Some(e) } else { None })
    }

    /// All text inside, trimmed.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s.trim().to_string()
    }

    fn collect_text(&self, s: &mut String) {
        for n in &self.children {
            match n {
                Node::Text(t) => s.push_str(t),
                Node::Element(e) => e.collect_text(s),
            }
        }
    }
}

/// Why a document could not be read: what, and the byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlError {
    pub message: &'static str,
    pub offset: usize,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn err<T>(&self, message: &'static str) -> Result<T, XmlError> {
        Err(XmlError { message, offset: self.pos })
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn eat(&mut self, prefix: &str) -> bool {
        if self.rest().starts_with(prefix) {
            self.pos += prefix.len();
            true
        } else {
            false
        }
    }

    /// Skip past `end`.
    fn skip_past(&mut self, end: &str, what: &'static str) -> Result<(), XmlError> {
        match self.rest().find(end) {
            Some(i) => {
                self.pos += i + end.len();
                Ok(())
            }
            None => self.err(what),
        }
    }

    fn skip_space(&mut self) {
        let n = self.rest().len() - self.rest().trim_start().len();
        self.pos += n;
    }

    /// Skip comments, processing instructions, the declaration and white
    /// space between top-level things.
    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_space();
            if self.eat("<!--") {
                self.skip_past("-->", "unterminated comment")?;
            } else if self.eat("<?") {
                self.skip_past("?>", "unterminated processing instruction")?;
            } else if self.rest().starts_with("<!DOCTYPE") {
                // Up to the matching '>', past an internal subset in [ ].
                let mut depth = 0i32;
                for (i, c) in self.rest().char_indices() {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        '>' if depth <= 0 => {
                            self.pos += i + 1;
                            break;
                        }
                        _ => {}
                    }
                    if i + c.len_utf8() == self.rest().len() {
                        return self.err("unterminated DOCTYPE");
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<&'a str, XmlError> {
        let r = self.rest();
        let end = r.find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/' | '<')).unwrap_or(r.len());
        if end == 0 {
            return self.err("expected a name");
        }
        self.pos += end;
        Ok(&r[..end])
    }

    fn element(&mut self, depth: usize) -> Result<Element, XmlError> {
        if depth > MAX_DEPTH {
            return self.err("elements nested too deeply");
        }
        if !self.eat("<") {
            return self.err("expected '<'");
        }
        let mut e = Element { name: self.name()?.to_string(), ..Element::default() };
        loop {
            self.skip_space();
            if self.eat("/>") {
                return Ok(e);
            }
            if self.eat(">") {
                break;
            }
            let name = self.name()?.to_string();
            self.skip_space();
            if !self.eat("=") {
                return self.err("expected '=' after an attribute name");
            }
            self.skip_space();
            let quote = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => return self.err("expected a quoted attribute value"),
            };
            self.pos += 1;
            let Some(end) = self.rest().find(quote) else { return self.err("unterminated attribute value") };
            let value =
                decode(&self.rest()[..end]).ok_or(XmlError { message: "bad character reference", offset: self.pos })?;
            self.pos += end + 1;
            e.attrs.push((name, value));
        }
        // Content up to the end tag.
        loop {
            if self.eat("</") {
                let name = self.name()?;
                if name != e.name {
                    return self.err("end tag does not match");
                }
                self.skip_space();
                if !self.eat(">") {
                    return self.err("expected '>'");
                }
                return Ok(e);
            } else if self.eat("<!--") {
                self.skip_past("-->", "unterminated comment")?;
            } else if self.eat("<![CDATA[") {
                let Some(end) = self.rest().find("]]>") else { return self.err("unterminated CDATA") };
                e.children.push(Node::Text(self.rest()[..end].to_string()));
                self.pos += end + 3;
            } else if self.eat("<?") {
                self.skip_past("?>", "unterminated processing instruction")?;
            } else if self.rest().starts_with('<') {
                let child = self.element(depth + 1)?;
                e.children.push(Node::Element(child));
            } else if self.rest().is_empty() {
                return self.err("unterminated element");
            } else {
                let end = self.rest().find('<').unwrap_or(self.rest().len());
                let text = decode(&self.rest()[..end])
                    .ok_or(XmlError { message: "bad character reference", offset: self.pos })?;
                self.pos += end;
                e.children.push(Node::Text(text));
            }
        }
    }
}

/// Replace character and entity references.
fn decode(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let end = rest.find(';')?;
        let name = &rest[..end];
        rest = &rest[end + 1..];
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let n = if let Some(hex) = name.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()?
                } else {
                    name.strip_prefix('#')?.parse().ok()?
                };
                char::from_u32(n)?
            }
        };
        out.push(c);
    }
    out.push_str(rest);
    Some(out)
}

/// Parse a document: its root element.
pub fn parse(text: &str) -> Result<Element, XmlError> {
    let mut p = Parser { s: text.strip_prefix('\u{feff}').unwrap_or(text), pos: 0 };
    p.skip_misc()?;
    let root = p.element(0)?;
    p.skip_misc()?;
    if !p.rest().is_empty() {
        return p.err("content after the root element");
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_document() {
        let doc = r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd" [ <!ENTITY x "y"> ]>
<!-- a comment -->
<fontconfig>
  <dir prefix='xdg'>fonts &amp; more</dir>
  <empty/>
  <!-- inside -->
  <a b="&lt;&#65;&#x42;&gt;"><![CDATA[<raw>]]></a>
</fontconfig>
"#;
        let root = parse(doc).unwrap();
        assert_eq!(root.name, "fontconfig");
        let kids: Vec<&Element> = root.elements().collect();
        assert_eq!(kids.len(), 3);
        assert_eq!(kids[0].attr("prefix"), Some("xdg"));
        assert_eq!(kids[0].text(), "fonts & more");
        assert_eq!(kids[1].name, "empty");
        assert_eq!(kids[2].attr("b"), Some("<AB>"));
        assert_eq!(kids[2].text(), "<raw>");
    }

    #[test]
    fn rejects_broken_documents() {
        for bad in [
            "",
            "<a>",
            "<a></b>",
            "<a b=c></a>",
            "<a>&bogus;</a>",
            "<a></a><b></b>",
            "<!-- open",
            "<a b='1></a>",
            "<!DOCTYPE x",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep: String = "<a>".repeat(100) + &"</a>".repeat(100);
        assert!(parse(&deep).is_err());
    }
}
