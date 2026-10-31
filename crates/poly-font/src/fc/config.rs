//! `fonts.conf`, as the fontconfig user documentation describes it
//! ("Configuration File Format"): the parts Poly uses.
//!
//! - `<dir>`, `<cachedir>` and `<include>` (with their `prefix` and
//!   `ignore_missing` attributes) are recorded; reading the files and
//!   directories they name is the caller's business (this crate does no
//!   I/O). [`Config::merge`] adds an included file's configuration.
//! - `<alias>` rewrites the family list: for each family the pattern
//!   names that the alias names, the `<prefer>` families go before it,
//!   the `<accept>` families after it, and the `<default>` families at the
//!   end of the list.
//! - `<match target="pattern">` is understood in one form, the one the
//!   documentation's example uses to give unknown families a generic
//!   fallback: `<test qual="all" name="family" compare="not_eq">` tests
//!   and an `<edit name="family" mode="append_last">` of strings. Other
//!   `<match>` and `<selectfont>` rules are counted in
//!   [`Config::ignored`] and otherwise ignored.
//!
//! Rules apply in document order, as fontconfig applies them.

use super::pattern::{Pattern, Value};
use super::xml::{self, Element, XmlError};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// An `<include>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Include {
    pub path: String,
    pub ignore_missing: bool,
    /// The `prefix` attribute (`xdg`, `default`, `cwd`, `relative`), if any.
    pub prefix: Option<String>,
}

/// A `<dir>` or `<cachedir>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    pub path: String,
    pub prefix: Option<String>,
}

/// An `<alias>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alias {
    pub family: Vec<String>,
    pub prefer: Vec<String>,
    pub accept: Vec<String>,
    pub default: Vec<String>,
}

/// A family-list rewrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Alias(Alias),
    /// Append `families` unless the pattern already names one of `unless`.
    AppendFamily {
        unless: Vec<String>,
        families: Vec<String>,
    },
}

/// A configuration: one file, or several merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub dirs: Vec<Dir>,
    pub cache_dirs: Vec<Dir>,
    pub includes: Vec<Include>,
    pub rules: Vec<Rule>,
    /// Elements this crate does not interpret.
    pub ignored: usize,
}

/// Why a configuration could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Xml(XmlError),
    /// The root element is not `<fontconfig>`.
    NotFontconfig,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConfigError::Xml(e) => write!(f, "{e}"),
            ConfigError::NotFontconfig => f.write_str("the root element is not <fontconfig>"),
        }
    }
}

/// Family names compare without case and blanks ("DejaVu Sans" ==
/// "dejavusans"), as fontconfig's do.
pub fn family_eq(a: &str, b: &str) -> bool {
    let mut x = a.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase);
    let mut y = b.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase);
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(p), Some(q)) if p == q => {}
            _ => return false,
        }
    }
}

fn families(e: &Element) -> Vec<String> {
    e.elements().filter(|c| c.name == "family").map(Element::text).filter(|t| !t.is_empty()).collect()
}

impl Config {
    /// Parse a `fonts.conf` document.
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let root = xml::parse(text).map_err(ConfigError::Xml)?;
        if root.name != "fontconfig" {
            return Err(ConfigError::NotFontconfig);
        }
        let mut c = Config::default();
        for e in root.elements() {
            let prefix = e.attr("prefix").map(str::to_string);
            match e.name.as_str() {
                "dir" => c.dirs.push(Dir { path: e.text(), prefix }),
                "cachedir" => c.cache_dirs.push(Dir { path: e.text(), prefix }),
                "include" => c.includes.push(Include {
                    path: e.text(),
                    ignore_missing: e.attr("ignore_missing") == Some("yes"),
                    prefix,
                }),
                "alias" => {
                    let mut a = Alias { family: families(e), ..Alias::default() };
                    for list in e.elements() {
                        match list.name.as_str() {
                            "prefer" => a.prefer.extend(families(list)),
                            "accept" => a.accept.extend(families(list)),
                            "default" => a.default.extend(families(list)),
                            _ => {}
                        }
                    }
                    c.rules.push(Rule::Alias(a));
                }
                "match" => match append_rule(e) {
                    Some(r) => c.rules.push(r),
                    None => c.ignored += 1,
                },
                "description" | "config" => {}
                _ => c.ignored += 1,
            }
        }
        Ok(c)
    }

    /// Add another file's configuration (an `<include>`d one) after this.
    pub fn merge(&mut self, other: Config) {
        self.dirs.extend(other.dirs);
        self.cache_dirs.extend(other.cache_dirs);
        self.includes.extend(other.includes);
        self.rules.extend(other.rules);
        self.ignored += other.ignored;
    }

    /// Apply the rules to a pattern's family list.
    pub fn substitute(&self, p: &mut Pattern) {
        let mut fams: Vec<String> = p.strings("family").map(str::to_string).collect();
        for rule in &self.rules {
            match rule {
                Rule::Alias(a) => {
                    let mut out: Vec<String> = Vec::with_capacity(fams.len());
                    let mut matched = false;
                    for f in &fams {
                        if a.family.iter().any(|af| family_eq(af, f)) {
                            matched = true;
                            out.extend(a.prefer.iter().cloned());
                            out.push(f.clone());
                            out.extend(a.accept.iter().cloned());
                        } else {
                            out.push(f.clone());
                        }
                    }
                    if matched {
                        out.extend(a.default.iter().cloned());
                    }
                    fams = out;
                }
                Rule::AppendFamily { unless, families } => {
                    if !fams.iter().any(|f| unless.iter().any(|u| family_eq(u, f))) {
                        fams.extend(families.iter().cloned());
                    }
                }
            }
            // Keep the list bounded however the rules chain.
            fams.truncate(64);
        }
        p.set("family", fams.into_iter().map(Value::Str).collect());
    }
}

/// The documented fallback rule: tests that every family differs from
/// some names, then an `append_last` of family strings.
fn append_rule(m: &Element) -> Option<Rule> {
    if m.attr("target").is_some_and(|t| t != "pattern") {
        return None;
    }
    let mut unless = Vec::new();
    let mut families = Vec::new();
    for e in m.elements() {
        match e.name.as_str() {
            "test" => {
                if e.attr("name") != Some("family")
                    || e.attr("qual") != Some("all")
                    || e.attr("compare") != Some("not_eq")
                {
                    return None;
                }
                unless.extend(e.elements().filter(|s| s.name == "string").map(Element::text));
            }
            "edit" => {
                if e.attr("name") != Some("family") || e.attr("mode") != Some("append_last") {
                    return None;
                }
                families.extend(e.elements().filter(|s| s.name == "string").map(Element::text));
            }
            _ => return None,
        }
    }
    (!families.is_empty()).then_some(Rule::AppendFamily { unless, families })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_ignore_case_and_blanks() {
        assert!(family_eq("DejaVu Sans", "dejavusans"));
        assert!(family_eq("DejaVu Sans", " DEJAVU  SANS "));
        assert!(!family_eq("DejaVu Sans", "DejaVu Sans Mono"));
    }

    #[test]
    fn aliases_and_the_fallback_rule() {
        let conf = r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <dir>/usr/share/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <cachedir>/var/cache/fontconfig</cachedir>
  <match target="pattern">
    <test qual="all" name="family" compare="not_eq"><string>sans-serif</string></test>
    <test qual="all" name="family" compare="not_eq"><string>serif</string></test>
    <edit name="family" mode="append_last"><string>sans-serif</string></edit>
  </match>
  <match target="font"><edit name="antialias"><bool>true</bool></edit></match>
  <alias><family>sans-serif</family><prefer><family>A Sans</family></prefer><accept><family>B Sans</family></accept></alias>
  <alias><family>Helvetica</family><default><family>sans-serif</family></default></alias>
  <include ignore_missing="yes">conf.d</include>
</fontconfig>"#;
        let c = Config::parse(conf).unwrap();
        assert_eq!(
            c.dirs,
            [
                Dir { path: "/usr/share/fonts".into(), prefix: None },
                Dir { path: "fonts".into(), prefix: Some("xdg".into()) }
            ]
        );
        assert_eq!(c.cache_dirs.len(), 1);
        assert_eq!(c.includes, [Include { path: "conf.d".into(), ignore_missing: true, prefix: None }]);
        assert_eq!(c.rules.len(), 3);
        assert_eq!(c.ignored, 1);
        let fams = |s: &str| {
            let mut p = Pattern::parse(s).unwrap();
            c.substitute(&mut p);
            p.strings("family").map(str::to_string).collect::<Vec<_>>()
        };
        // An unknown family gets the fallback, which the alias expands.
        assert_eq!(fams("Foo"), ["Foo", "A Sans", "sans-serif", "B Sans"]);
        // A generic family is left alone by the fallback rule.
        assert_eq!(fams("serif"), ["serif"]);
        assert_eq!(fams("Sans-Serif"), ["A Sans", "Sans-Serif", "B Sans"]);
        // No family at all: the fallback still applies.
        assert_eq!(fams(":bold"), ["A Sans", "sans-serif", "B Sans"]);
        // Helvetica's default comes at the end, after the fallback, but the
        // sans-serif alias came before it in the file and does not see it.
        assert_eq!(fams("Helvetica"), ["Helvetica", "A Sans", "sans-serif", "B Sans", "sans-serif"]);
        assert_eq!(Config::parse("<other/>"), Err(ConfigError::NotFontconfig));
        assert!(matches!(Config::parse("<fontconfig>"), Err(ConfigError::Xml(_))));
        let mut merged = c.clone();
        merged.merge(c.clone());
        assert_eq!(merged.rules.len(), 6);
    }
}
