//! fontconfig patterns and their text form, as the fontconfig user
//! documentation describes them ("Font Names", "Font Properties",
//! "Constants"):
//!
//! ```text
//! <families>-<point sizes>:<name1>=<values1>:<name2>=<values2>...
//! ```
//!
//! Families and sizes are comma-separated lists, as are the values of a
//! property; each part is optional. (A `-` followed by anything but
//! numbers is taken as part of the family name, so `sans-serif` names the
//! generic family without an escape.) A bare word after a colon is a
//! constant naming a property's value (`:bold` is `weight=200`), and
//! constants may stand for values too (`slant=italic`). A backslash
//! escapes `\`, `-`, `:` and `,` in family names and `\`, `=`, `_`, `:`
//! and `,` in values.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i32),
    Double(f64),
    Bool(bool),
}

impl Value {
    /// The value as a number (integers and doubles).
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Int(i) => Some(f64::from(i)),
            Value::Double(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => {
                for c in s.chars() {
                    if matches!(c, '\\' | '=' | '_' | ':' | ',') {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                Ok(())
            }
            Value::Int(i) => write!(f, "{i}"),
            Value::Double(d) => write!(f, "{d}"),
            Value::Bool(b) => f.write_str(if *b { "True" } else { "False" }),
        }
    }
}

/// The type of a standard property (fontconfig user documentation, "Font
/// Properties").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Str,
    Int,
    Double,
    Bool,
}

/// The standard properties and their types.
const PROPERTIES: &[(&str, Kind)] = &[
    ("family", Kind::Str),
    ("familylang", Kind::Str),
    ("style", Kind::Str),
    ("stylelang", Kind::Str),
    ("fullname", Kind::Str),
    ("fullnamelang", Kind::Str),
    ("slant", Kind::Int),
    ("weight", Kind::Int),
    ("width", Kind::Int),
    ("size", Kind::Double),
    ("aspect", Kind::Double),
    ("pixelsize", Kind::Double),
    ("spacing", Kind::Int),
    ("foundry", Kind::Str),
    ("antialias", Kind::Bool),
    ("hintstyle", Kind::Int),
    ("hinting", Kind::Bool),
    ("verticallayout", Kind::Bool),
    ("autohint", Kind::Bool),
    ("globaladvance", Kind::Bool),
    ("file", Kind::Str),
    ("index", Kind::Int),
    ("rasterizer", Kind::Str),
    ("outline", Kind::Bool),
    ("scalable", Kind::Bool),
    ("dpi", Kind::Double),
    ("rgba", Kind::Int),
    ("scale", Kind::Double),
    ("minspace", Kind::Bool),
    ("charwidth", Kind::Int),
    ("charheight", Kind::Int),
    ("matrix", Kind::Str),
    ("lang", Kind::Str),
    ("fontversion", Kind::Int),
    ("capability", Kind::Str),
    ("fontformat", Kind::Str),
    ("embolden", Kind::Bool),
    ("embeddedbitmap", Kind::Bool),
    ("decorative", Kind::Bool),
    ("lcdfilter", Kind::Int),
    ("namelang", Kind::Str),
    ("fontfeatures", Kind::Str),
    ("prgname", Kind::Str),
    ("postscriptname", Kind::Str),
    ("color", Kind::Bool),
    ("symbol", Kind::Bool),
    ("fontvariations", Kind::Str),
    ("variable", Kind::Bool),
    ("fonthashint", Kind::Bool),
    ("order", Kind::Int),
];

/// The type of a property, if it is a standard one.
pub fn kind(name: &str) -> Option<Kind> {
    PROPERTIES.iter().find(|p| p.0 == name).map(|p| p.1)
}

/// The constants: name, property, value (fontconfig user documentation,
/// "Constants").
pub const CONSTANTS: &[(&str, &str, i32)] = &[
    ("thin", "weight", 0),
    ("extralight", "weight", 40),
    ("ultralight", "weight", 40),
    ("light", "weight", 50),
    ("demilight", "weight", 55),
    ("semilight", "weight", 55),
    ("book", "weight", 75),
    ("regular", "weight", 80),
    ("normal", "weight", 80),
    ("medium", "weight", 100),
    ("demibold", "weight", 180),
    ("semibold", "weight", 180),
    ("bold", "weight", 200),
    ("extrabold", "weight", 205),
    ("ultrabold", "weight", 205),
    ("black", "weight", 210),
    ("heavy", "weight", 210),
    ("extrablack", "weight", 215),
    ("ultrablack", "weight", 215),
    ("roman", "slant", 0),
    ("italic", "slant", 100),
    ("oblique", "slant", 110),
    ("ultracondensed", "width", 50),
    ("extracondensed", "width", 63),
    ("condensed", "width", 75),
    ("semicondensed", "width", 87),
    ("normal", "width", 100),
    ("semiexpanded", "width", 113),
    ("expanded", "width", 125),
    ("extraexpanded", "width", 150),
    ("ultraexpanded", "width", 200),
    ("proportional", "spacing", 0),
    ("dual", "spacing", 90),
    ("mono", "spacing", 100),
    ("charcell", "spacing", 110),
    ("unknown", "rgba", 0),
    ("rgb", "rgba", 1),
    ("bgr", "rgba", 2),
    ("vrgb", "rgba", 3),
    ("vbgr", "rgba", 4),
    ("none", "rgba", 5),
    ("lcdnone", "lcdfilter", 0),
    ("lcddefault", "lcdfilter", 1),
    ("lcdlight", "lcdfilter", 2),
    ("lcdlegacy", "lcdfilter", 3),
    ("hintnone", "hintstyle", 0),
    ("hintslight", "hintstyle", 1),
    ("hintmedium", "hintstyle", 2),
    ("hintfull", "hintstyle", 3),
];

/// A constant by name (the first of its name: `normal` is a weight).
pub fn constant(name: &str) -> Option<(&'static str, i32)> {
    CONSTANTS.iter().find(|c| c.0.eq_ignore_ascii_case(name)).map(|c| (c.1, c.2))
}

/// A constant by name for a given property (`normal` for `width` is 100).
fn constant_for(property: &str, name: &str) -> Option<i32> {
    CONSTANTS.iter().find(|c| c.1 == property && c.0.eq_ignore_ascii_case(name)).map(|c| c.2)
}

/// Why a pattern's text could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// A bare word that is no constant.
    UnknownConstant(String),
    /// A value that does not fit its property's type.
    BadValue { property: String, value: String },
    /// `=` with no property name before it.
    EmptyName,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownConstant(c) => write!(f, "unknown constant {c:?}"),
            ParseError::BadValue { property, value } => write!(f, "bad value {value:?} for {property}"),
            ParseError::EmptyName => f.write_str("a property without a name"),
        }
    }
}

/// A set of properties, each with a list of values (most preferred first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pattern {
    props: Vec<(String, Vec<Value>)>,
}

/// Split at unescaped `sep`, keeping escapes in the pieces.
fn split_unescaped(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            out.push(&s[start..i]);
            start = i + 1;
        }
    }
    out.push(&s[start..]);
    out
}

/// The position of the first unescaped `c`.
fn find_unescaped(s: &str, c: char) -> Option<usize> {
    let mut escaped = false;
    for (i, ch) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == c {
            return Some(i);
        }
    }
    None
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut escaped = false;
    for c in s.chars() {
        if escaped || c != '\\' {
            out.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    out
}

/// A boolean as fontconfig writes them: true/false, yes/no, on/off, 1/0.
fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "on" | "1" => Some(true),
        "false" | "f" | "no" | "n" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a value for `property`.
pub fn parse_value(property: &str, raw: &str) -> Option<Value> {
    let s = unescape(raw);
    let t = s.trim();
    match kind(property) {
        Some(Kind::Str) => Some(Value::Str(s)),
        Some(Kind::Int) => t
            .parse::<i32>()
            .ok()
            .map(Value::Int)
            .or_else(|| constant_for(property, t).map(Value::Int))
            .or_else(|| t.parse::<f64>().ok().filter(|d| d.is_finite()).map(Value::Double)),
        Some(Kind::Double) => t.parse::<f64>().ok().filter(|d| d.is_finite()).map(Value::Double),
        Some(Kind::Bool) => parse_bool(t).map(Value::Bool),
        None => Some(if let Ok(i) = t.parse::<i32>() {
            Value::Int(i)
        } else if let Some(d) = t.parse::<f64>().ok().filter(|d| d.is_finite()) {
            Value::Double(d)
        } else if let Some(c) = constant_for(property, t) {
            Value::Int(c)
        } else if let Some(b) = parse_bool(t) {
            Value::Bool(b)
        } else {
            Value::Str(s)
        }),
    }
}

impl Pattern {
    pub fn new() -> Pattern {
        Pattern::default()
    }

    /// Parse a font name.
    pub fn parse(text: &str) -> Result<Pattern, ParseError> {
        let mut p = Pattern::new();
        let parts = split_unescaped(text, ':');
        let head = parts[0];
        // Point sizes after the first unescaped '-', if they are numbers.
        let mut sizes = Vec::new();
        let mut families = head;
        if let Some(i) = find_unescaped(head, '-') {
            let parsed: Option<Vec<f64>> = split_unescaped(&head[i + 1..], ',')
                .into_iter()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<f64>().ok().filter(|d| d.is_finite()))
                .collect();
            if let Some(list) = parsed {
                sizes = list;
                families = &head[..i];
            }
        }
        for f in split_unescaped(families, ',') {
            let f = unescape(f);
            let f = f.trim();
            if !f.is_empty() {
                p.add("family", Value::Str(f.to_string()));
            }
        }
        for d in sizes {
            p.add("size", Value::Double(d));
        }
        for part in &parts[1..] {
            if part.trim().is_empty() {
                continue;
            }
            match find_unescaped(part, '=') {
                Some(i) => {
                    let name = part[..i].trim();
                    if name.is_empty() {
                        return Err(ParseError::EmptyName);
                    }
                    for raw in split_unescaped(&part[i + 1..], ',') {
                        if raw.trim().is_empty() {
                            continue;
                        }
                        let v = parse_value(name, raw).ok_or_else(|| ParseError::BadValue {
                            property: name.to_string(),
                            value: raw.to_string(),
                        })?;
                        p.add(name, v);
                    }
                }
                None => {
                    let word = unescape(part);
                    let word = word.trim();
                    let (prop, v) = constant(word).ok_or_else(|| ParseError::UnknownConstant(word.to_string()))?;
                    p.add(prop, Value::Int(v));
                }
            }
        }
        Ok(p)
    }

    /// Append a value to a property's list.
    pub fn add(&mut self, name: &str, v: Value) {
        match self.props.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1.push(v),
            None => self.props.push((name.to_string(), alloc::vec![v])),
        }
    }

    /// Replace a property's values.
    pub fn set(&mut self, name: &str, values: Vec<Value>) {
        match self.props.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1 = values,
            None => self.props.push((name.to_string(), values)),
        }
        self.props.retain(|p| !p.1.is_empty());
    }

    pub fn remove(&mut self, name: &str) {
        self.props.retain(|p| p.0 != name);
    }

    /// A property's values (empty if absent).
    pub fn get(&self, name: &str) -> &[Value] {
        self.props.iter().find(|p| p.0 == name).map_or(&[], |p| &p.1)
    }

    pub fn has(&self, name: &str) -> bool {
        !self.get(name).is_empty()
    }

    /// A property's first string value.
    pub fn get_str(&self, name: &str) -> Option<&str> {
        self.get(name).iter().find_map(Value::as_str)
    }

    /// A property's first numeric value.
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        self.get(name).iter().find_map(Value::as_f64)
    }

    /// A property's first boolean value.
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.get(name).iter().find_map(|v| if let Value::Bool(b) = v { Some(*b) } else { None })
    }

    /// The string values of a property.
    pub fn strings<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a str> + 'a {
        self.get(name).iter().filter_map(Value::as_str)
    }

    /// Property names in the order they were added.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.props.iter().map(|p| p.0.as_str())
    }

    /// Only the named properties (in the order named).
    pub fn filter(&self, names: &[&str]) -> Pattern {
        let mut p = Pattern::new();
        for n in names {
            let v = self.get(n);
            if !v.is_empty() {
                p.set(n, v.to_vec());
            }
        }
        p
    }
}

/// The text form, which [`Pattern::parse`] reads back: families (escaped),
/// `-` and point sizes, then `:name=values` for everything else.
impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for v in self.get("family") {
            if !first {
                f.write_str(",")?;
            }
            first = false;
            if let Value::Str(s) = v {
                for c in s.chars() {
                    if matches!(c, '\\' | '-' | ':' | ',') {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
            }
        }
        let sizes = self.get("size");
        if !sizes.is_empty() {
            f.write_str("-")?;
            for (i, s) in sizes.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{s}")?;
            }
        }
        for (name, values) in &self.props {
            if name == "family" || name == "size" {
                continue;
            }
            write!(f, ":{name}=")?;
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{v}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec;

    #[test]
    fn parses_the_documented_examples() {
        // "Times-12" is 12 point Times.
        let p = Pattern::parse("Times-12").unwrap();
        assert_eq!(p.get("family"), &[Value::Str("Times".into())]);
        assert_eq!(p.get("size"), &[Value::Double(12.0)]);
        // "Times-12:bold" is 12 point Times Bold.
        let p = Pattern::parse("Times-12:bold").unwrap();
        assert_eq!(p.get("weight"), &[Value::Int(200)]);
        // "Courier:italic" is the default size of Courier Italic.
        let p = Pattern::parse("Courier:italic").unwrap();
        assert_eq!(p.get("slant"), &[Value::Int(100)]);
        assert!(!p.has("size"));
        // "Monospace:matrix=1 .1 0 1" keeps the matrix as a string here.
        let p = Pattern::parse("Monospace:matrix=1 .1 0 1").unwrap();
        assert_eq!(p.get_str("matrix"), Some("1 .1 0 1"));
    }

    #[test]
    fn lists_constants_and_types() {
        let p = Pattern::parse(
            "DejaVu Sans,sans-serif:style=Bold:pixelsize=12:weight=bold,80:spacing=mono:antialias=false:index=1",
        )
        .unwrap();
        assert_eq!(p.strings("family").collect::<Vec<_>>(), ["DejaVu Sans", "sans-serif"]);
        assert_eq!(p.get_str("style"), Some("Bold"));
        assert_eq!(p.get("pixelsize"), &[Value::Double(12.0)]);
        assert_eq!(p.get("weight"), &[Value::Int(200), Value::Int(80)]);
        assert_eq!(p.get("spacing"), &[Value::Int(100)]);
        assert_eq!(p.get_bool("antialias"), Some(false));
        assert_eq!(p.get("index"), &[Value::Int(1)]);
        // "normal" as a width is the width constant.
        assert_eq!(Pattern::parse(":width=normal").unwrap().get("width"), &[Value::Int(100)]);
        // Unknown properties guess their type.
        let p = Pattern::parse(":foo=3:bar=2.5:baz=true:qux=hello").unwrap();
        assert_eq!(p.get("foo"), &[Value::Int(3)]);
        assert_eq!(p.get("bar"), &[Value::Double(2.5)]);
        assert_eq!(p.get("baz"), &[Value::Bool(true)]);
        assert_eq!(p.get("qux"), &[Value::Str("hello".into())]);
    }

    #[test]
    fn errors() {
        assert_eq!(Pattern::parse(":sideways"), Err(ParseError::UnknownConstant("sideways".into())));
        assert_eq!(
            Pattern::parse(":weight=heavyish"),
            Err(ParseError::BadValue { property: "weight".into(), value: "heavyish".into() })
        );
        // "-big" is no size: part of the name.
        assert_eq!(Pattern::parse("Times-big").unwrap().get_str("family"), Some("Times-big"));
        assert_eq!(Pattern::parse(r"A\-B-9,10").unwrap().get("size"), &[Value::Double(9.0), Value::Double(10.0)]);
        assert_eq!(Pattern::parse(":=3"), Err(ParseError::EmptyName));
        assert_eq!(Pattern::parse(""), Ok(Pattern::new()));
        assert_eq!(Pattern::parse(":"), Ok(Pattern::new()));
    }

    #[test]
    fn round_trips_through_text() {
        let mut p = Pattern::new();
        p.add("family", Value::Str("A-B: C, D\\".into()));
        p.add("family", Value::Str("Other".into()));
        p.add("size", Value::Double(10.5));
        p.add("style", Value::Str("x=y_z:w,v".into()));
        p.add("weight", Value::Int(200));
        p.add("antialias", Value::Bool(true));
        let text = format!("{p}");
        assert_eq!(text, r"A\-B\: C\, D\\,Other-10.5:style=x\=y\_z\:w\,v:weight=200:antialias=True");
        assert_eq!(Pattern::parse(&text).unwrap(), p);
        assert_eq!(format!("{}", p.filter(&["style", "family"])), r"A\-B\: C\, D\\,Other:style=x\=y\_z\:w\,v");
        let mut q = p.clone();
        q.set("weight", vec![]);
        assert!(!q.has("weight"));
        q.remove("family");
        assert_eq!(format!("{q}"), r"-10.5:style=x\=y\_z\:w\,v:antialias=True");
    }
}
