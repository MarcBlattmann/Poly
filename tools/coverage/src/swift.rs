//! The public Swift API of the macOS SDK, by name (ADR 0187).
//!
//! The input is the SDK's `.swiftinterface` files: the textual,
//! documented description of a Swift module's public API that Apple ships
//! beside each module (`arm64e-apple-macos.swiftinterface`). They are read
//! lexically, with no Swift tooling; only the *names* of public
//! declarations are taken, never types, bodies or documentation.
//!
//! Entry ids, below `macos/api/<component>/` (see [`component`] for the
//! component of a module):
//!
//! ```text
//! swift:Foundation.URL                           a type
//! swift:Foundation.URL.appendingPathComponent(_:)  a method: name and argument labels
//! swift:Foundation.URL.path                      a property
//! swift:Foundation.URL.init(fileURLWithPath:)    an initializer
//! swift:Foundation.URL.subscript(_:)             a subscript
//! swift:Swift.Array.Index                        a nested or associated type, typealias
//! swift:Swift.Optional.some                      an enum case
//! swift:Swift.min(_:_:)                          a free function
//! ```
//!
//! The module is the one that *declares* the API: a method that
//! `Foundation` adds to `Swift.String` is `swift:Foundation.String.<name>`.
//! Overloads that differ only in types are one entry (the map holds names,
//! not signatures). Declarations marked `@_spi`, unavailable on macOS, or
//! not `public`/`open` are left out; members of a protocol are public.

use crate::model::{Entry, Layer, Os};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const MACOS_SWIFT: &str = "macos-swift-interface";

/// The interface this importer reads for each module.
pub const INTERFACE_FILE: &str = "arm64e-apple-macos.swiftinterface";

/// The module's name from the `-module-name` flag in the header comment.
pub fn module_name(text: &str) -> Option<String> {
    for line in text.lines().take(12) {
        if let Some(flags) = line.strip_prefix("// swift-module-flags:") {
            let mut words = flags.split_whitespace();
            while let Some(w) = words.next() {
                if w == "-module-name" {
                    return words.next().map(str::to_string);
                }
            }
        }
    }
    None
}

/// The text without comments, and with the inside of string literals
/// emptied (so a brace or a slash in a string cannot confuse the reader).
/// Line breaks stay.
pub fn sanitize(text: &str) -> String {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block(u32),
        Str,
        Multi,
    }
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut s = S::Code;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let next = c.get(i + 1).copied();
        match s {
            S::Code => {
                if ch == '/' && next == Some('/') {
                    s = S::Line;
                    i += 2;
                } else if ch == '/' && next == Some('*') {
                    s = S::Block(1);
                    i += 2;
                } else if ch == '"' && next == Some('"') && c.get(i + 2) == Some(&'"') {
                    s = S::Multi;
                    out.push_str("\"\"");
                    i += 3;
                } else if ch == '"' {
                    s = S::Str;
                    out.push('"');
                    i += 1;
                } else {
                    out.push(ch);
                    i += 1;
                }
            }
            S::Line => {
                if ch == '\n' {
                    out.push('\n');
                    s = S::Code;
                }
                i += 1;
            }
            S::Block(d) => {
                if ch == '/' && next == Some('*') {
                    s = S::Block(d + 1);
                    i += 2;
                } else if ch == '*' && next == Some('/') {
                    s = if d == 1 { S::Code } else { S::Block(d - 1) };
                    i += 2;
                } else {
                    if ch == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            S::Str => {
                if ch == '\\' {
                    i += 2;
                } else if ch == '"' {
                    out.push('"');
                    s = S::Code;
                    i += 1;
                } else if ch == '\n' {
                    // An unterminated string ends at the line.
                    out.push('\n');
                    s = S::Code;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            S::Multi => {
                if ch == '"' && next == Some('"') && c.get(i + 2) == Some(&'"') {
                    s = S::Code;
                    i += 3;
                } else {
                    if ch == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn is_operator_char(c: char) -> bool {
    "/=-+!*%<>&|^~?.".contains(c)
}

/// `(` minus `)` in the text.
fn paren_balance(s: &str) -> i32 {
    s.chars().map(|c| i32::from(c == '(') - i32::from(c == ')')).sum()
}

/// `{` minus `}` in the text.
fn brace_delta(s: &str) -> i32 {
    s.chars().map(|c| i32::from(c == '{') - i32::from(c == '}')).sum()
}

/// The text up to the matching close of the bracket that `s` starts with
/// (`open` and `close` are the pair), and the rest after it. `s` starts
/// with `open`. `->` does not count as a closing angle bracket.
fn balanced(s: &str, open: char, close: char) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close && !(close == '>' && prev == '-') {
            depth -= 1;
            if depth == 0 {
                return Some((&s[open.len_utf8()..i], &s[i + c.len_utf8()..]));
            }
        }
        prev = c;
    }
    None
}

/// Splits at the commas that are not inside brackets (`->` aside).
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut prev) = (0i32, 0usize, ' ');
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if prev != '-' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        prev = c;
    }
    out.push(&s[start..]);
    out
}

/// The leading attributes of a line (`@inlinable`, `@available(...)`) and
/// what follows them.
fn split_attributes(line: &str) -> (Vec<String>, &str) {
    let mut attrs = Vec::new();
    let mut rest = line.trim_start();
    while rest.starts_with('@') {
        let end = rest[1..].find(|c: char| !(is_ident(c) || c == '.')).map_or(rest.len(), |i| i + 1);
        let mut attr_end = end;
        if rest[end..].starts_with('(') {
            match balanced(&rest[end..], '(', ')') {
                Some((_, after)) => attr_end = rest.len() - after.len(),
                None => attr_end = rest.len(),
            }
        }
        attrs.push(rest[..attr_end].to_string());
        rest = rest[attr_end..].trim_start();
    }
    (attrs, rest)
}

/// Whether an attribute says the declaration does not exist on macOS.
fn unavailable_on_macos(attr: &str) -> bool {
    let Some(args) = attr.strip_prefix("@available(").and_then(|a| a.strip_suffix(')')) else { return false };
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    parts.first().is_some_and(|p| *p == "*" || *p == "macOS") && parts.contains(&"unavailable")
}

const MODIFIERS: &[&str] = &[
    "public",
    "open",
    "package",
    "internal",
    "private",
    "fileprivate",
    "final",
    "static",
    "mutating",
    "nonmutating",
    "override",
    "convenience",
    "required",
    "indirect",
    "dynamic",
    "nonisolated",
    "optional",
    "prefix",
    "postfix",
    "infix",
    "lazy",
    "weak",
    "unowned",
    "_const",
    "borrowing",
    "consuming",
    "distributed",
    "isolated",
    "sending",
    "__owned",
    "__shared",
    "_resultDependsOn",
];

const KEYWORDS: &[&str] = &[
    "struct",
    "class",
    "enum",
    "actor",
    "protocol",
    "extension",
    "func",
    "var",
    "let",
    "init",
    "subscript",
    "typealias",
    "associatedtype",
    "case",
    "deinit",
    "macro",
    "operator",
    "precedencegroup",
    "import",
];

/// First word of `s` and the rest after it (and any blanks).
fn word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    let end = s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// The modifiers, the keyword and the rest of a declaration (after its
/// attributes). `class` is a modifier when a member keyword follows it.
fn split_declaration(rest: &str) -> (Vec<&str>, &str, &str) {
    let mut mods = Vec::new();
    let mut s = rest;
    loop {
        let (w, after) = word(s);
        // `unowned(unsafe)` and similar take a parenthesised argument.
        let after_arg = if MODIFIERS.contains(&w) && after.starts_with('(') {
            balanced(after, '(', ')').map_or(after, |(_, r)| r)
        } else {
            after
        };
        let is_class_modifier = w == "class" && {
            let (next, _) = word(after);
            [
                "func",
                "var",
                "let",
                "subscript",
                "final",
                "override",
                "public",
                "open",
                "internal",
                "private",
                "nonisolated",
            ]
            .contains(&next)
        };
        if MODIFIERS.contains(&w) || is_class_modifier {
            mods.push(w);
            s = after_arg;
        } else {
            return (mods, w, after);
        }
    }
}

/// The name that follows a type keyword: `Array<Element> : P` gives `Array`.
fn type_name(tail: &str) -> Option<String> {
    let (w, _) = word(tail.trim_start().trim_start_matches('`'));
    (!w.is_empty()).then(|| w.to_string())
}

/// The type an `extension` extends, without generic arguments, module
/// qualifiers or conformances: `Swift::Array<Element> : P where ...` gives `Array`.
fn extended_type(tail: &str) -> Option<String> {
    let mut s = tail.split('{').next().unwrap_or("").trim();
    if let Some(i) = s.find(" where ") {
        s = &s[..i];
    }
    // A single ':' (a conformance), not the '::' of a module qualifier.
    let b = s.as_bytes();
    let cut = (0..b.len()).find(|&i| b[i] == b':' && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':'));
    let s = cut.map_or(s, |i| &s[..i]).trim();
    // Generic arguments, innermost first.
    let mut name = String::new();
    let mut depth = 0;
    let mut prev = ' ';
    for c in s.chars() {
        if c == '<' {
            depth += 1;
        } else if c == '>' && prev != '-' && depth > 0 {
            depth -= 1;
        } else if depth == 0 {
            name.push(c);
        }
        prev = c;
    }
    let name = name.trim().replace('`', "");
    let name = name.rsplit("::").next().unwrap_or("").trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// `name(label:label:)` of a function-like declaration, from the text after
/// its keyword. `subscript` argument names are labels only when written
/// twice.
fn function_name(tail: &str, subscript: bool) -> Option<String> {
    let s = tail.trim_start();
    let (name, mut rest) = if subscript {
        ("subscript".to_string(), s)
    } else if s.starts_with(|c: char| is_operator_char(c)) {
        let end = s.find(|c: char| !is_operator_char(c)).unwrap_or(s.len());
        (s[..end].to_string(), &s[end..])
    } else {
        let (w, after) = word(s.trim_start_matches('`'));
        let after = after.strip_prefix('`').unwrap_or(after);
        (w.to_string(), after)
    };
    if name.is_empty() {
        return None;
    }
    rest = rest.trim_start();
    // Failable `init?` and `init!`.
    rest = rest.strip_prefix(['?', '!']).unwrap_or(rest).trim_start();
    if rest.starts_with('<') {
        rest = balanced(rest, '<', '>')?.1.trim_start();
    }
    let (params, _) = balanced(rest, '(', ')')?;
    let mut labels = String::new();
    if !params.trim().is_empty() {
        for p in split_top_level(params) {
            let names = p.split(':').next().unwrap_or("");
            let words: Vec<&str> = names.split_whitespace().collect();
            let first = words.first().copied().unwrap_or("_");
            let label = if subscript && words.len() < 2 { "_" } else { first };
            labels.push_str(&format!("{}:", label.trim_matches('`')));
        }
    }
    Some(format!("{name}({labels})"))
}

/// The first identifier of `s`, without backticks.
fn first_ident(s: &str) -> Option<String> {
    let s = s.trim_start().trim_start_matches('`');
    let end = s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    (end > 0).then(|| s[..end].to_string())
}

#[derive(Debug)]
enum Ctx {
    /// Inside a type (or an extension of one): the members belong to `path`.
    Type { path: String, protocol: bool },
    /// Inside something whose lines are not read (a body, a type that is not public).
    Skip,
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() { name.to_string() } else { format!("{path}.{name}") }
}

/// The names of the public declarations of one interface, without the
/// module (`URL.path`, `min(_:_:)`).
pub fn public_names(text: &str) -> Vec<String> {
    let clean = sanitize(text);
    let lines: Vec<&str> = clean.lines().collect();
    let mut out = Vec::new();
    let mut stack: Vec<(Ctx, i32)> = Vec::new();
    let mut depth = 0i32;
    let mut pending: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut line = lines[i].trim().to_string();
        i += 1;
        // A declaration may wrap its parameters over several lines.
        while paren_balance(&line) > 0 && i < lines.len() {
            line.push(' ');
            line.push_str(lines[i].trim());
            i += 1;
        }
        let delta = brace_delta(&line);
        let skipping = matches!(stack.last(), Some((Ctx::Skip, _)));
        if !skipping && !line.is_empty() && !line.starts_with('#') {
            let (path, in_protocol) = match stack.last() {
                Some((Ctx::Type { path, protocol }, _)) => (path.clone(), *protocol),
                _ => (String::new(), false),
            };
            let (attrs, rest) = split_attributes(&line);
            if rest.is_empty() {
                pending.extend(attrs);
            } else {
                let attrs: Vec<String> = pending.drain(..).chain(attrs).collect();
                let pushed = declaration(rest, &attrs, &path, in_protocol, delta, &mut out);
                if delta > 0 {
                    stack.push((pushed.unwrap_or(Ctx::Skip), depth));
                }
            }
        }
        depth += delta;
        while let Some((_, open)) = stack.last() {
            if depth <= *open {
                stack.pop();
            } else {
                break;
            }
        }
    }
    out
}

/// Handles one declaration line: appends the names it declares and returns
/// the context its `{ ... }` opens (only asked for when the line opens one).
fn declaration(
    rest: &str,
    attrs: &[String],
    path: &str,
    in_protocol: bool,
    delta: i32,
    out: &mut Vec<String>,
) -> Option<Ctx> {
    let (mods, keyword, tail) = split_declaration(rest);
    if !KEYWORDS.contains(&keyword) {
        return None;
    }
    let hidden = attrs.iter().any(|a| a.starts_with("@_spi(") || a.starts_with("@_spi ") || unavailable_on_macos(a))
        || mods.iter().any(|m| matches!(*m, "internal" | "private" | "fileprivate" | "package"));
    let public = mods.iter().any(|m| matches!(*m, "public" | "open"));
    let visible = !hidden && (public || in_protocol);
    match keyword {
        "struct" | "class" | "enum" | "actor" | "protocol" => {
            let name = type_name(tail)?;
            if !visible {
                return None;
            }
            let full = join(path, &name);
            out.push(full.clone());
            (delta > 0).then_some(Ctx::Type { path: full, protocol: keyword == "protocol" })
        }
        "extension" => {
            let ty = extended_type(tail)?;
            if hidden {
                return None;
            }
            Some(Ctx::Type { path: join(path, &ty), protocol: false })
        }
        "func" | "init" | "subscript" if visible => {
            let tail = if keyword == "init" { format!("init{tail}") } else { tail.to_string() };
            let name = if keyword == "init" {
                function_name(&tail, false)
            } else {
                function_name(&tail, keyword == "subscript")
            }?;
            out.push(join(path, &name));
            None
        }
        "var" | "let" | "typealias" | "associatedtype" if visible => {
            let name = first_ident(tail)?;
            out.push(join(path, &name));
            None
        }
        "case" if !path.is_empty() && !hidden => {
            for part in split_top_level(tail) {
                if let Some(name) = first_ident(part) {
                    out.push(join(path, &name));
                }
            }
            None
        }
        _ => None,
    }
}

/// The component of a module's interface: the framework that holds it, or,
/// for the modules of `usr/lib/swift`, the Swift library of that module
/// (`libswiftMetal.dylib`; `Swift` is `libswiftCore.dylib`), or
/// `<Module>.swiftmodule` when the map has no such library.
pub fn component(interface: &Path, sdk: &Path, module: &str, known: &BTreeSet<String>) -> Option<String> {
    let rel = interface.strip_prefix(sdk).ok()?;
    let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if comps.first().map(String::as_str) == Some("System") && comps.get(1).map(String::as_str) == Some("iOSSupport") {
        return None;
    }
    if let Some(fw) = comps.iter().find(|c| c.ends_with(".framework")) {
        return Some(fw.clone());
    }
    if module == "Swift" {
        return Some("libswiftCore.dylib".to_string());
    }
    let lib = format!("libswift{module}.dylib");
    Some(if known.contains(&lib) { lib } else { format!("{module}.swiftmodule") })
}

/// Every `arm64e-apple-macos` interface in the SDK, in path order.
pub fn find_interfaces(sdk: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let Ok(read) = std::fs::read_dir(dir) else { return Ok(()) };
        for entry in read {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if entry.file_name() != "PrivateFrameworks" {
                    walk(&path, out)?;
                }
            } else if entry.file_name() == INTERFACE_FILE {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for dir in [
        "System/Library/Frameworks",
        "System/Library/SubFrameworks",
        "System/Cryptexes/OS/System/Library/Frameworks",
        "usr/lib/swift",
    ] {
        walk(&sdk.join(dir), &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// The entries of one interface.
pub fn entries_from_interface(component: &str, module: &str, text: &str) -> Vec<Entry> {
    public_names(text)
        .into_iter()
        .map(|n| Entry::new(Os::Macos, Layer::Api, component, &format!("swift:{module}.{n}"), MACOS_SWIFT))
        .collect()
}

/// The Swift API of every module in the SDK. `known` holds the components
/// the map already has (to place the modules of `usr/lib/swift`).
pub fn macos_swift(sdk: &Path, known: &BTreeSet<String>) -> Result<Vec<Entry>, String> {
    if !sdk.join("SDKSettings.json").exists() && !sdk.join("SDKSettings.plist").exists() {
        return Err(format!("{} does not look like a macOS SDK (no SDKSettings)", sdk.display()));
    }
    let files = find_interfaces(sdk)?;
    if files.is_empty() {
        return Err(format!("no {INTERFACE_FILE} files under {}", sdk.display()));
    }
    let mut out = Vec::new();
    let mut modules = 0;
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let dir_name = f
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().trim_end_matches(".swiftmodule").to_string())
            .unwrap_or_default();
        let module = module_name(&text).unwrap_or(dir_name);
        let Some(comp) = component(f, sdk, &module, known) else { continue };
        out.extend(entries_from_interface(&comp, &module, &text));
        modules += 1;
    }
    eprintln!("read {modules} Swift interfaces");
    Ok(crate::import::dedup(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERFACE: &str = r#"// swift-interface-format-version: 1.0
// swift-compiler-version: Apple Swift version 6.4.2
// swift-module-flags: -target arm64e-apple-macosx27.2 -enable-library-evolution -module-name Demo -Osize
import Swift
/// A documented free function with a brace in { a comment }.
@inlinable public func min<T>(_ x: T, _ y: T) -> T where T : Swift::Comparable {
  let s = "{ not a brace"
  return y < x ? y : x
}
public func open(path: Swift::String, mode: Swift::Int = 0) throws -> Swift::Int
@available(macOS 10.15, *)
@frozen public struct Point : Swift::Hashable {
  public var x: Swift::Double
  public let y: Swift::Double
  public static let zero: Demo::Point
  public init(x: Swift::Double, y: Swift::Double)
  public init?(string: Swift::String)
  public subscript(index: Swift::Int) -> Swift::Double { get set }
  public subscript(row row: Swift::Int, column: Swift::Int) -> Swift::Double {
    get { return 0 }
  }
  @inlinable public static func + (lhs: Demo::Point, rhs: Demo::Point) -> Demo::Point {
    return lhs
  }
  public func distance(to other: Demo::Point, using metric: ((Swift::Double, Swift::Double) -> Swift::Double)? = nil) -> Swift::Double
  internal func hidden()
  @usableFromInline internal var cache: Swift::Int
  @_spi(Private) public func spi()
  @available(*, unavailable, message: "gone")
  public func gone()
  @available(iOS, unavailable)
  public func iosOnly()
  public enum Kind : Swift::Int {
    case flat
    case round, square = 3
    indirect case nested(inner: Demo::Point.Kind, Swift::Int)
    public var label: Swift::String {
      get
    }
  }
  public typealias Scalar = Swift::Double
}
extension Demo::Point {
  public func moved(by dx: Swift::Double, _ dy: Swift::Double) -> Demo::Point
  public struct Box<Content> where Content : Swift::Equatable {
    public var content: Content
  }
}
extension Swift::Array<Element> : Demo::Drawable where Element : Demo::Drawable {
  public func draw(in context: Demo::Context)
  #if compiler(>=5.3) && $Foo
  public func drawFast()
  #else
  public func drawSlow()
  #endif
}
extension Demo::Point : Swift::Sendable {}
public protocol Drawable : AnyObject {
  associatedtype Canvas
  var bounds: Swift::Double { get }
  func draw(in context: Demo::Context)
  init(frame: Swift::Double)
  @objc optional func tick()
}
open class Window : ObjectiveC::NSObject {
  @objc public class func make() -> Demo::Window
  public final class var shared: Demo::Window { get }
  public init(contentRect: Swift::Double)
  deinit
  private func secret()
}
internal struct Private {
  public func notReally()
}
"#;

    #[test]
    fn module_flag() {
        assert_eq!(module_name(INTERFACE).as_deref(), Some("Demo"));
        assert_eq!(module_name("// nothing\n"), None);
    }

    #[test]
    fn comments_and_strings_are_removed() {
        let s = sanitize("a // b { c\n/* x { /* y */ } */ d \"e { }\" f \"\"\"\nm { \n\"\"\" g\n");
        assert_eq!(s.matches('{').count(), 0);
        assert!(s.contains("a ") && s.contains(" d ") && s.contains(" g"));
        assert_eq!(s.matches('\n').count(), 4);
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(function_name("min<T>(_ x: T, _ y: T) -> T", false).as_deref(), Some("min(_:_:)"));
        assert_eq!(function_name("open(path: String, mode: Int = 0)", false).as_deref(), Some("open(path:mode:)"));
        assert_eq!(function_name("== (lhs: A, rhs: A) -> Bool", false).as_deref(), Some("==(lhs:rhs:)"));
        assert_eq!(function_name("<= <T>(lhs: T, rhs: T)", false).as_deref(), Some("<=(lhs:rhs:)"));
        assert_eq!(
            function_name("init?(fileURLWithPath path: String)", false).as_deref(),
            Some("init(fileURLWithPath:)")
        );
        assert_eq!(function_name("f()", false).as_deref(), Some("f()"));
        assert_eq!(function_name("g(_: Int, h: (A) -> B, i j: [K: V])", false).as_deref(), Some("g(_:h:i:)"));
        assert_eq!(function_name("`default`(value: Int)", false).as_deref(), Some("default(value:)"));
        assert_eq!(function_name("(index: Int)", true).as_deref(), Some("subscript(_:)"));
        assert_eq!(function_name("(row r: Int, column c: Int)", true).as_deref(), Some("subscript(row:column:)"));
    }

    #[test]
    fn extended_types() {
        assert_eq!(extended_type("Swift::Array<Element> : P where Element : Q {").as_deref(), Some("Array"));
        assert_eq!(extended_type("Swift::Dictionary<K, V>.Keys {").as_deref(), Some("Dictionary.Keys"));
        assert_eq!(extended_type("Demo::Point : Swift::Sendable {}").as_deref(), Some("Point"));
        assert_eq!(extended_type("Foundation::URL where S : Swift::Sequence {").as_deref(), Some("URL"));
        assert_eq!(extended_type("(A, B) -> C where A : P {").as_deref(), Some("(A, B) -> C"));
    }

    #[test]
    fn the_public_names_of_an_interface() {
        let names = public_names(INTERFACE);
        let want = [
            "min(_:_:)",
            "open(path:mode:)",
            "Point",
            "Point.x",
            "Point.y",
            "Point.zero",
            "Point.init(x:y:)",
            "Point.init(string:)",
            "Point.subscript(_:)",
            "Point.subscript(row:_:)",
            "Point.+(lhs:rhs:)",
            "Point.distance(to:using:)",
            "Point.iosOnly()",
            "Point.Kind",
            "Point.Kind.flat",
            "Point.Kind.round",
            "Point.Kind.square",
            "Point.Kind.nested",
            "Point.Kind.label",
            "Point.Scalar",
            "Point.moved(by:_:)",
            "Point.Box",
            "Point.Box.content",
            "Array.draw(in:)",
            "Array.drawFast()",
            "Array.drawSlow()",
            "Drawable",
            "Drawable.Canvas",
            "Drawable.bounds",
            "Drawable.draw(in:)",
            "Drawable.init(frame:)",
            "Drawable.tick()",
            "Window",
            "Window.make()",
            "Window.shared",
            "Window.init(contentRect:)",
        ];
        assert_eq!(names, want, "\nleft:  {names:#?}");
    }

    #[test]
    fn entries_carry_the_module() {
        let e = entries_from_interface("Demo.framework", "Demo", INTERFACE);
        assert_eq!(e[0].id, "macos/api/Demo.framework/swift:Demo.min(_:_:)");
        assert!(e.iter().all(|e| e.sources == [MACOS_SWIFT]));
    }

    #[test]
    fn components_of_modules() {
        let sdk = Path::new("/sdk");
        let known: BTreeSet<String> = ["libswiftMetal.dylib".to_string()].into();
        let c = |p: &str, m: &str| component(&Path::new("/sdk").join(p), sdk, m, &known);
        assert_eq!(
            c("System/Library/Frameworks/SwiftUI.framework/Versions/A/Modules/SwiftUI.swiftmodule/x", "SwiftUI")
                .as_deref(),
            Some("SwiftUI.framework")
        );
        assert_eq!(
            c("System/Cryptexes/OS/System/Library/Frameworks/WebKit.framework/Versions/A/Modules/WebKit.swiftmodule/x", "WebKit")
                .as_deref(),
            Some("WebKit.framework")
        );
        assert_eq!(c("usr/lib/swift/Swift.swiftmodule/x", "Swift").as_deref(), Some("libswiftCore.dylib"));
        assert_eq!(c("usr/lib/swift/Metal.swiftmodule/x", "Metal").as_deref(), Some("libswiftMetal.dylib"));
        assert_eq!(c("usr/lib/swift/Odd.swiftmodule/x", "Odd").as_deref(), Some("Odd.swiftmodule"));
        assert_eq!(c("System/iOSSupport/System/Library/Frameworks/X.framework/Modules/X.swiftmodule/x", "X"), None);
    }
}
