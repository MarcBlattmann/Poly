//! `cargo xtask wayland [--check]`: turn the Wayland protocol XML files in
//! `third_party/` into the typed request and event definitions
//! `poly-wayland` compiles in (ADR 0106).
//!
//! The XML files are the protocol's specification ("The Wayland Protocol",
//! "Protocol Design"): unmodified copies of the wayland 1.26.0 and
//! wayland-protocols 1.50 releases (MIT; `THIRD_PARTY.md`), pinned here by
//! SHA-256 ([`PROTOCOLS`]) so an edit to one is noticed. For every
//! interface the generator writes a module with:
//!
//! - `NAME`, `VERSION` and `INTERFACE`, the `poly_wayland::wire::Interface` table the
//!   decoder walks: each request and event with its opcode (its index),
//!   the version that added it, whether it is a destructor, and its
//!   arguments (type, interface, `allow-null`);
//! - one struct per `<enum>`, a `u32` newtype with an associated constant
//!   per entry (bitfields and plain enums alike, so unknown values survive);
//! - `Request` and `Event`, an enum each with a variant per message whose
//!   fields are the typed arguments, and `opcode`, `from_args` and
//!   `into_args` to convert from and to the wire's argument lists.
//!
//! The modules are checked in (`crates/poly-wayland/src/protocol/`), so a
//! build needs neither the XML nor this tool; `cargo xtask ci lint` runs
//! this with `--check` to make sure they still match their sources. Only
//! the `summary` attributes become doc comments; the long descriptions stay
//! in the XML.

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// A protocol file, the module generated from it, and the SHA-256 of the
/// file as released.
struct ProtocolFile {
    xml: &'static str,
    module: &'static str,
    sha256: &'static str,
}

/// From the releases' tarballs `wayland-1.26.0.tar.xz` (SHA-256
/// `64176eaa46e4969903e286f8e5ef8331affc17fdf03ac9b58381d2b23162b7a3`) and
/// `wayland-protocols-1.50.tar.xz` (SHA-256
/// `4460c20cfcf68b994a5ee206ed15fc19ad312f16c96075186e83b0e3f4aa48a5`),
/// both as published with their `.sha256sum` files on
/// gitlab.freedesktop.org.
const PROTOCOLS: [ProtocolFile; 6] = [
    ProtocolFile {
        xml: "third_party/wayland/wayland.xml",
        module: "wayland",
        sha256: "cc860987e54f8d85c940e97fa1270c69b6e4ad31fbcf5a7f00107ce1157f5e07",
    },
    ProtocolFile {
        xml: "third_party/wayland-protocols/xdg-shell/xdg-shell.xml",
        module: "xdg_shell",
        sha256: "8cd3980d0783e1f55d12901ac51533204af71487cc4dc69ee00bb7096647d9de",
    },
    ProtocolFile {
        xml: "third_party/wayland-protocols/xdg-decoration/xdg-decoration-unstable-v1.xml",
        module: "xdg_decoration_unstable_v1",
        sha256: "68753c4a85a28659e3aa1fe0f138329a8bf3dff0b4789f4663a2c7fedbfecf2f",
    },
    ProtocolFile {
        xml: "third_party/wayland-protocols/viewporter/viewporter.xml",
        module: "viewporter",
        sha256: "dcb12279a03746301fe490aaed4b38a403485a925abfce2ccfceb644e104fe71",
    },
    ProtocolFile {
        xml: "third_party/wayland-protocols/presentation-time/presentation-time.xml",
        module: "presentation_time",
        sha256: "0a9e4ee351aada90af9d165a540b86ad9d85f740ce87cf8c26490dd1ae701b5f",
    },
    ProtocolFile {
        xml: "third_party/wayland-protocols/linux-dmabuf/linux-dmabuf-v1.xml",
        module: "linux_dmabuf_v1",
        sha256: "d4feffb8c0a3dbdecfff141f4d65d88d91ff2b022cb343c988b73e3c358d9a76",
    },
];

/// Where the modules go.
const OUT_DIR: &str = "crates/poly-wayland/src/protocol";

/// `wayland [--check]`
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [a] if a == "--check" => true,
        _ => return Err("usage: cargo xtask wayland [--check]".into()),
    };
    let mut protocols = Vec::new();
    for p in &PROTOCOLS {
        let path = root.join(p.xml);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(sum) = sha256(&path)
            && sum != p.sha256
        {
            return Err(format!("{}: SHA-256 {sum}, not the released file's {}", p.xml, p.sha256));
        }
        let text = String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8", p.xml))?;
        let protocol = parse_protocol(&text).map_err(|e| format!("{}: {e}", p.xml))?;
        protocols.push((p, protocol));
    }
    let mut files = vec![("mod.rs".to_string(), generate_mod(&protocols))];
    for (p, protocol) in &protocols {
        files.push((format!("{}.rs", p.module), generate_protocol(p, protocol)));
    }
    let mut stale = Vec::new();
    for (name, source) in files {
        let source = rustfmt(root, &source)?;
        let out = root.join(OUT_DIR).join(&name);
        let current = std::fs::read_to_string(&out).unwrap_or_default();
        if current == source {
            continue;
        }
        if check {
            stale.push(format!("{OUT_DIR}/{name}"));
        } else {
            std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&out, source).map_err(|e| format!("{}: {e}", out.display()))?;
            eprintln!("==> wrote {OUT_DIR}/{name}");
        }
    }
    if stale.is_empty() { Ok(()) } else { Err(format!("out of date, run `cargo xtask wayland`: {}", stale.join(", "))) }
}

/// The SHA-256 of a file (`sha256sum`), or `None` without the tool.
fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.split_whitespace().next()?.to_string())
}

/// `source` formatted with the repository's `rustfmt.toml`.
fn rustfmt(root: &Path, source: &str) -> Result<String, String> {
    use std::io::Write as _;
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout", "--config-path"])
        .arg(root.join("rustfmt.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("rustfmt: {e}"))?;
    child.stdin.take().unwrap().write_all(source.as_bytes()).map_err(|e| format!("rustfmt: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!("rustfmt failed on generated code: {}", String::from_utf8_lossy(&out.stderr)));
    }
    String::from_utf8(out.stdout).map_err(|_| "rustfmt: output not UTF-8".into())
}

// ---------------------------------------------------------------------------
// A small XML reader: elements and attributes, enough for protocol files
// (no DTDs; text content is skipped, the generator only uses attributes).

/// An element with its attributes and child elements.
#[derive(Debug, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or_else(|| format!("unterminated entity in {s:?}"))? + i;
        let entity = &rest[i + 1..end];
        let c = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    entity.strip_prefix('#').and_then(|d| d.parse().ok())
                };
                code.and_then(char::from_u32).ok_or_else(|| format!("unknown entity &{entity};"))?
            }
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Parse an XML document; returns its root element.
pub fn parse_xml(text: &str) -> Result<Element, String> {
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut rest = text;
    let line = |rest: &str| text[..text.len() - rest.len()].lines().count().max(1);
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        if let Some(r) = rest.strip_prefix("<!--") {
            let end = r.find("-->").ok_or_else(|| format!("line {}: unterminated comment", line(rest)))?;
            rest = &r[end + 3..];
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").ok_or_else(|| format!("line {}: unterminated CDATA", line(rest)))?;
            rest = &r[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            let end = rest.find('>').ok_or_else(|| format!("line {}: unterminated declaration", line(rest)))?;
            rest = &rest[end + 1..];
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>').ok_or_else(|| format!("line {}: unterminated end tag", line(rest)))?;
            let name = r[..end].trim();
            let done = stack.pop().filter(|e| e.name == name && !stack.is_empty());
            let done = done.ok_or_else(|| format!("line {}: unexpected </{name}>", line(rest)))?;
            stack.last_mut().unwrap().children.push(done);
            rest = &r[end + 1..];
        } else {
            let (element, empty, after) = start_tag(&rest[1..]).map_err(|e| format!("line {}: {e}", line(rest)))?;
            rest = after;
            if empty {
                stack.last_mut().unwrap().children.push(element);
            } else {
                stack.push(element);
            }
        }
    }
    let mut document = stack.pop().unwrap();
    if !stack.is_empty() {
        return Err(format!("unclosed element <{}>", document.name));
    }
    match document.children.len() {
        1 => Ok(document.children.pop().unwrap()),
        n => Err(format!("{n} root elements")),
    }
}

/// A start tag after its `<`: the element, whether it is empty (`/>`), and
/// the text after it.
fn start_tag(s: &str) -> Result<(Element, bool, &str), String> {
    let name_end = s.find(|c: char| c.is_whitespace() || c == '>' || c == '/').ok_or("unterminated tag")?;
    let mut element = Element { name: s[..name_end].to_string(), ..Element::default() };
    let mut rest = &s[name_end..];
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("/>") {
            return Ok((element, true, r));
        }
        if let Some(r) = rest.strip_prefix('>') {
            return Ok((element, false, r));
        }
        let eq = rest.find('=').ok_or_else(|| format!("attribute without a value in <{}>", element.name))?;
        let name = rest[..eq].trim().to_string();
        let r = rest[eq + 1..].trim_start();
        let quote = r.chars().next().filter(|c| *c == '"' || *c == '\'').ok_or("unquoted attribute")?;
        let end = r[1..].find(quote).ok_or("unterminated attribute")? + 1;
        element.attrs.push((name, unescape(&r[1..end])?));
        rest = &r[end + 1..];
    }
}

// ---------------------------------------------------------------------------
// The protocol model.

#[derive(Debug)]
pub struct Protocol {
    pub name: String,
    pub interfaces: Vec<Iface>,
}

#[derive(Debug)]
pub struct Iface {
    pub name: String,
    pub version: u32,
    pub summary: String,
    pub requests: Vec<Message>,
    pub events: Vec<Message>,
    pub enums: Vec<Enum>,
}

#[derive(Debug)]
pub struct Message {
    pub name: String,
    pub since: u32,
    pub destructor: bool,
    pub summary: String,
    pub args: Vec<Arg>,
}

#[derive(Debug)]
pub struct Arg {
    pub name: String,
    pub kind: String,
    pub interface: Option<String>,
    pub nullable: bool,
    pub summary: String,
}

#[derive(Debug)]
pub struct Enum {
    pub name: String,
    pub summary: String,
    pub bitfield: bool,
    pub entries: Vec<(String, u32, String)>,
}

fn summary(e: &Element) -> String {
    let s = e.attr("summary").or_else(|| e.children_named("description").next().and_then(|d| d.attr("summary")));
    s.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn number(s: &str) -> Result<u32, String> {
    let parsed = match s.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => s.parse(),
    };
    parsed.map_err(|_| format!("bad number {s:?}"))
}

fn required<'a>(e: &'a Element, attr: &str) -> Result<&'a str, String> {
    e.attr(attr).ok_or_else(|| format!("<{}> without {attr}", e.name))
}

const KINDS: [&str; 8] = ["int", "uint", "fixed", "string", "object", "new_id", "array", "fd"];

fn parse_message(e: &Element) -> Result<Message, String> {
    let name = required(e, "name")?.to_string();
    let mut args = Vec::new();
    for a in e.children_named("arg") {
        let kind = required(a, "type")?.to_string();
        if !KINDS.contains(&kind.as_str()) {
            return Err(format!("{name}: argument type {kind:?}"));
        }
        args.push(Arg {
            name: required(a, "name")?.to_string(),
            kind,
            interface: a.attr("interface").map(str::to_string),
            nullable: a.attr("allow-null") == Some("true"),
            summary: summary(a),
        });
    }
    Ok(Message {
        since: e.attr("since").map(number).transpose()?.unwrap_or(1),
        destructor: e.attr("type") == Some("destructor"),
        summary: summary(e),
        args,
        name,
    })
}

/// The protocol a protocol file describes.
pub fn parse_protocol(text: &str) -> Result<Protocol, String> {
    let root = parse_xml(text)?;
    if root.name != "protocol" {
        return Err(format!("root element <{}>, not <protocol>", root.name));
    }
    let mut interfaces = Vec::new();
    for i in root.children_named("interface") {
        let name = required(i, "name")?.to_string();
        let mut iface = Iface {
            version: number(required(i, "version")?)?,
            summary: summary(i),
            requests: Vec::new(),
            events: Vec::new(),
            enums: Vec::new(),
            name,
        };
        for c in &i.children {
            match c.name.as_str() {
                "request" => iface.requests.push(parse_message(c).map_err(|e| format!("{}: {e}", iface.name))?),
                "event" => iface.events.push(parse_message(c).map_err(|e| format!("{}: {e}", iface.name))?),
                "enum" => {
                    let mut entries = Vec::new();
                    for entry in c.children_named("entry") {
                        let value = number(required(entry, "value")?)?;
                        entries.push((required(entry, "name")?.to_string(), value, summary(entry)));
                    }
                    iface.enums.push(Enum {
                        name: required(c, "name")?.to_string(),
                        summary: summary(c),
                        bitfield: c.attr("bitfield") == Some("true"),
                        entries,
                    });
                }
                _ => {}
            }
        }
        interfaces.push(iface);
    }
    Ok(Protocol { name: required(&root, "name")?.to_string(), interfaces })
}

// ---------------------------------------------------------------------------
// Code generation.

const KEYWORDS: [&str; 52] = [
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

fn field(name: &str) -> String {
    if KEYWORDS.contains(&name) { format!("{name}_") } else { name.to_string() }
}

fn camel(name: &str) -> String {
    let mut out = String::new();
    for part in name.split('_').filter(|p| !p.is_empty()) {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) { format!("_{out}") } else { out }
}

fn constant(name: &str) -> String {
    let upper = name.to_uppercase();
    if upper.starts_with(|c: char| c.is_ascii_digit()) { format!("_{upper}") } else { upper }
}

/// A doc comment line, markdown's link and HTML characters escaped.
fn doc(indent: &str, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut escaped = String::new();
    for c in text.chars() {
        if matches!(c, '[' | ']' | '<' | '>') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("{indent}/// {escaped}\n")
}

fn quoted(s: &str) -> String {
    format!("{s:?}")
}

/// The Rust type of an argument.
fn rust_type(a: &Arg) -> &'static str {
    match (a.kind.as_str(), a.nullable) {
        ("int", _) => "i32",
        ("uint", _) => "u32",
        ("fixed", _) => "Fixed",
        ("string", false) => "String",
        ("string", true) => "Option<String>",
        ("object", false) => "ObjectId",
        ("object", true) => "Option<ObjectId>",
        ("new_id", _) if a.interface.is_none() => "NewObject",
        ("new_id", _) => "ObjectId",
        ("array", _) => "Vec<u8>",
        _ => "Fd",
    }
}

/// The `Args` call that takes an argument.
fn take(a: &Arg) -> &'static str {
    match (a.kind.as_str(), a.nullable) {
        ("int", _) => "a.int()?",
        ("uint", _) => "a.uint()?",
        ("fixed", _) => "a.fixed()?",
        ("string", false) => "a.string()?",
        ("string", true) => "a.string_opt()?",
        ("object", false) => "a.object()?",
        ("object", true) => "a.object_opt()?",
        ("new_id", _) if a.interface.is_none() => "a.dyn_new_id()?",
        ("new_id", _) => "a.new_id()?",
        ("array", _) => "a.array()?",
        _ => "a.fd()?",
    }
}

/// The `Arg` an argument becomes, given its variable.
fn wrap(a: &Arg, var: &str) -> String {
    match (a.kind.as_str(), a.nullable) {
        ("int", _) => format!("Arg::Int({var})"),
        ("uint", _) => format!("Arg::Uint({var})"),
        ("fixed", _) => format!("Arg::Fixed({var})"),
        ("string", false) => format!("Arg::Str(Some({var}))"),
        ("string", true) => format!("Arg::Str({var})"),
        ("object", false) => format!("Arg::Object(Some({var}))"),
        ("object", true) => format!("Arg::Object({var})"),
        ("new_id", _) if a.interface.is_none() => format!("Arg::DynNewId({var})"),
        ("new_id", _) => format!("Arg::NewId({var})"),
        ("array", _) => format!("Arg::Array({var})"),
        _ => format!("Arg::Fd({var})"),
    }
}

fn kind_variant(kind: &str) -> &'static str {
    match kind {
        "int" => "Int",
        "uint" => "Uint",
        "fixed" => "Fixed",
        "string" => "String",
        "object" => "Object",
        "new_id" => "NewId",
        "array" => "Array",
        _ => "Fd",
    }
}

fn message_table(out: &mut String, messages: &[Message]) {
    out.push_str("&[");
    for m in messages {
        let _ = write!(
            out,
            "MessageDesc {{ name: {}, since: {}, destructor: {}, args: &[",
            quoted(&m.name),
            m.since,
            m.destructor
        );
        for a in &m.args {
            let interface = a.interface.as_deref().map_or("None".to_string(), |i| format!("Some({})", quoted(i)));
            let _ = write!(
                out,
                "ArgDesc {{ name: {}, kind: ArgKind::{}, interface: {interface}, nullable: {} }},",
                quoted(&a.name),
                kind_variant(&a.kind),
                a.nullable
            );
        }
        out.push_str("] },");
    }
    out.push(']');
}

/// `Request` or `Event` and its conversions.
fn message_enum(out: &mut String, ty: &str, messages: &[Message]) {
    let _ = writeln!(out, "    #[derive(Clone, Debug, PartialEq)]\n    pub enum {ty} {{");
    for m in messages {
        out.push_str(&doc("        ", &m.summary));
        if m.args.is_empty() {
            let _ = writeln!(out, "        {},", camel(&m.name));
        } else {
            let _ = writeln!(out, "        {} {{", camel(&m.name));
            for a in &m.args {
                out.push_str(&doc("            ", &a.summary));
                let _ = writeln!(out, "            {}: {},", field(&a.name), rust_type(a));
            }
            out.push_str("        },\n");
        }
    }
    out.push_str("    }\n\n");
    let _ = writeln!(out, "    impl {ty} {{");
    if messages.is_empty() {
        out.push_str("        pub fn opcode(&self) -> u16 { match *self {} }\n");
        let _ = writeln!(
            out,
            "        pub fn from_args(opcode: u16, _args: Vec<Arg>) -> Result<{ty}, WireError> {{ Err(WireError::BadOpcode(opcode)) }}"
        );
        out.push_str("        pub fn into_args(self) -> Vec<Arg> { match self {} }\n    }\n");
        message_trait(out, ty);
        return;
    }
    out.push_str(
        "        /// The message's opcode.\n        pub fn opcode(&self) -> u16 {\n            match self {\n",
    );
    for (op, m) in messages.iter().enumerate() {
        let pattern = if m.args.is_empty() { String::new() } else { " { .. }".into() };
        let _ = writeln!(out, "                {ty}::{}{pattern} => {op},", camel(&m.name));
    }
    out.push_str("            }\n        }\n\n");
    let _ = writeln!(
        out,
        "        /// The message `opcode` with `args` (from [`crate::wire::decode_args`]).\n        pub fn from_args(opcode: u16, args: Vec<Arg>) -> Result<{ty}, WireError> {{"
    );
    let any_args = messages.iter().any(|m| !m.args.is_empty());
    let binding = if any_args { "mut a" } else { "a" };
    let _ = writeln!(out, "            let {binding} = Args::new(args);\n            let m = match opcode {{");
    for (op, m) in messages.iter().enumerate() {
        if m.args.is_empty() {
            let _ = writeln!(out, "                {op} => {ty}::{},", camel(&m.name));
        } else {
            let fields: Vec<String> = m.args.iter().map(|a| format!("{}: {}", field(&a.name), take(a))).collect();
            let _ = writeln!(out, "                {op} => {ty}::{} {{ {} }},", camel(&m.name), fields.join(", "));
        }
    }
    out.push_str("                _ => return Err(WireError::BadOpcode(opcode)),\n            };\n            a.end()?;\n            Ok(m)\n        }\n\n");
    out.push_str("        /// The message's arguments, for [`crate::wire::encode`].\n        pub fn into_args(self) -> Vec<Arg> {\n            match self {\n");
    for m in messages {
        if m.args.is_empty() {
            let _ = writeln!(out, "                {ty}::{} => Vec::new(),", camel(&m.name));
        } else {
            let names: Vec<String> = m.args.iter().map(|a| field(&a.name)).collect();
            let values: Vec<String> = m.args.iter().map(|a| wrap(a, &field(&a.name))).collect();
            let _ = writeln!(
                out,
                "                {ty}::{} {{ {} }} => alloc::vec![{}],",
                camel(&m.name),
                names.join(", "),
                values.join(", ")
            );
        }
    }
    out.push_str("            }\n        }\n    }\n");
    message_trait(out, ty);
}

/// `impl Message`, so the server sends any interface's events alike.
fn message_trait(out: &mut String, ty: &str) {
    let _ = writeln!(
        out,
        "\n    impl Message for {ty} {{\n        fn opcode(&self) -> u16 {{ {ty}::opcode(self) }}\n        fn into_args(self) -> Vec<Arg> {{ {ty}::into_args(self) }}\n    }}"
    );
}

fn generate_interface(out: &mut String, i: &Iface) {
    out.push_str(&doc("", &format!("`{}`: {}", i.name, i.summary)));
    let _ = writeln!(out, "pub mod {} {{\n    use crate::protocol::prelude::*;\n", i.name);
    let _ = writeln!(out, "    pub const NAME: &str = {};", quoted(&i.name));
    let _ = writeln!(out, "    pub const VERSION: u32 = {};\n", i.version);
    out.push_str("    pub static INTERFACE: Interface = Interface {\n        name: NAME,\n        version: VERSION,\n        requests: ");
    message_table(out, &i.requests);
    out.push_str(",\n        events: ");
    message_table(out, &i.events);
    out.push_str(",\n    };\n\n");
    for e in &i.enums {
        let what = if e.bitfield { "a bitfield" } else { "an enum" };
        out.push_str(&doc("    ", &format!("`{}` ({what}): {}", e.name, e.summary)));
        let ty = camel(&e.name);
        let _ = writeln!(
            out,
            "    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]\n    pub struct {ty}(pub u32);\n\n    impl {ty} {{"
        );
        for (name, value, summary) in &e.entries {
            out.push_str(&doc("        ", summary));
            let _ = writeln!(out, "        pub const {}: {ty} = {ty}({value:#x});", constant(name));
        }
        out.push_str("    }\n\n");
    }
    message_enum(out, "Request", &i.requests);
    out.push('\n');
    message_enum(out, "Event", &i.events);
    out.push_str("}\n\n");
}

const HEADER: &str = "// Generated by `cargo xtask wayland` (tools/xtask/src/wayland.rs) from";

fn generate_protocol(p: &ProtocolFile, protocol: &Protocol) -> String {
    let mut out = format!(
        "{HEADER}\n// {} (MIT; THIRD_PARTY.md). Do not edit.\n\n//! The `{}` protocol: {}.\n\n",
        p.xml,
        protocol.name,
        protocol.interfaces.iter().map(|i| format!("`{}`", i.name)).collect::<Vec<_>>().join(", ")
    );
    for i in &protocol.interfaces {
        generate_interface(&mut out, i);
    }
    out
}

fn generate_mod(protocols: &[(&ProtocolFile, Protocol)]) -> String {
    let mut out =
        format!("{HEADER}\n// the protocol XML files in third_party/ (MIT; THIRD_PARTY.md). Do not edit.\n\n");
    out.push_str(
        "//! The protocols, generated from their XML: one module per interface (all\n\
         //! re-exported here), each with its `INTERFACE` table, its enums, and its\n\
         //! `Request` and `Event` types.\n\n\
         #![allow(clippy::enum_variant_names)]\n\n",
    );
    out.push_str(
        "/// What the generated modules use.\npub mod prelude {\n    pub use crate::wire::{Arg, ArgDesc, ArgKind, Args, Fd, Fixed, Interface, Message, MessageDesc, NewObject, ObjectId, WireError};\n    pub use alloc::string::String;\n    pub use alloc::vec::Vec;\n}\n\n",
    );
    for (p, _) in protocols {
        let _ = writeln!(out, "pub mod {};", p.module);
    }
    out.push('\n');
    for (p, _) in protocols {
        let _ = writeln!(out, "pub use {}::*;", p.module);
    }
    out.push_str("\n/// Every interface of every protocol.\npub static INTERFACES: &[&crate::wire::Interface] = &[\n");
    for (_, protocol) in protocols {
        for i in &protocol.interfaces {
            let _ = writeln!(out, "    &{}::INTERFACE,", i.name);
        }
    }
    out.push_str(
        "];\n\n/// The interface called `name`.\npub fn interface(name: &str) -> Option<&'static crate::wire::Interface> {\n    INTERFACES.iter().copied().find(|i| i.name == name)\n}\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_comments_and_entities() {
        let xml = "<?xml version=\"1.0\"?>\n<!-- c <x> -->\n<protocol name='p'>\n  <interface name=\"a\" version=\"2\">\
                   <description summary=\"one &amp; two\">text &lt;here&gt;</description>\
                   <request name=\"r\" since=\"2\"><arg name=\"type\" type=\"new_id\"/></request>\
                   <enum name=\"e\" bitfield=\"true\"><entry name=\"90\" value=\"0x10\"/></enum>\
                   </interface></protocol>";
        let p = parse_protocol(xml).unwrap();
        assert_eq!(p.name, "p");
        let i = &p.interfaces[0];
        assert_eq!((i.name.as_str(), i.version, i.summary.as_str()), ("a", 2, "one & two"));
        assert_eq!((i.requests[0].since, i.requests[0].args[0].interface.as_deref()), (2, None));
        assert_eq!(i.enums[0].entries[0], ("90".to_string(), 16, String::new()));
        assert!(i.enums[0].bitfield);
        assert_eq!(
            (field("type"), camel("get_registry"), constant("90")),
            ("type_".into(), "GetRegistry".into(), "_90".into())
        );
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(parse_xml("<a><b></a>").is_err());
        assert!(parse_xml("<a>").is_err());
        assert!(parse_xml("<a x=1/>").is_err());
        assert!(parse_xml("<a/><b/>").is_err());
        assert!(parse_protocol("<protocol name=\"p\"><interface name=\"i\" version=\"x\"/></protocol>").is_err());
    }

    /// Every protocol file parses, and wl_surface looks as the book describes.
    #[test]
    fn parses_the_protocol_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap();
        for p in &PROTOCOLS {
            let text = std::fs::read_to_string(root.join(p.xml)).unwrap();
            let protocol = parse_protocol(&text).unwrap();
            assert!(!protocol.interfaces.is_empty(), "{}", p.xml);
        }
        let text = std::fs::read_to_string(root.join(PROTOCOLS[0].xml)).unwrap();
        let wayland = parse_protocol(&text).unwrap();
        let surface = wayland.interfaces.iter().find(|i| i.name == "wl_surface").unwrap();
        let names: Vec<&str> = surface.requests.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(&names[..3], ["destroy", "attach", "damage"]);
        assert!(surface.requests[0].destructor);
        assert!(surface.requests[1].args[0].nullable);
    }
}
