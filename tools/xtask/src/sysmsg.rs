//! `cargo xtask system-messages [--from <dir>]`: turn Microsoft Learn's
//! "System Error Codes" pages into the system message table `FormatMessageW`
//! serves (ADR 0101).
//!
//! The sources are the ten pages Learn's "System Error Codes" page links
//! (<https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499->
//! and the other ranges in [`PAGES`]). Each code is three paragraphs: the
//! constant's name in bold (after its anchors), `N (0xH)`, and the message
//! text; a few codes have a further paragraph (an older name, a note on old
//! Windows versions), which is documentation, not message text, and is
//! skipped. Tags inside the text (links, bold) are dropped, entities
//! decoded, runs of white space made one space.
//!
//! Without `--from` the pages are downloaded with `curl` into
//! `target/learn-system-error-codes/` (only read as data). They are not
//! pinned by hash: Learn's HTML changes with every page build, while the
//! texts do not. What is written is checked in, so a build needs neither
//! the pages nor this tool:
//!
//! - `crates/poly-kernelbase/data/system-messages.tsv`: the extracted data,
//!   one code per line: `code<TAB>name<TAB>text`, by code.
//! - `crates/poly-kernelbase/src/sysmsg.rs`: the same as a table the crate
//!   compiles in (codes, end offsets, one string).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const BASE: &str = "https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--";

/// The pages: the range in each page's name, inclusive.
const PAGES: &[(u32, u32)] = &[
    (0, 499),
    (500, 999),
    (1000, 1299),
    (1300, 1699),
    (1700, 3999),
    (4000, 5999),
    (6000, 8199),
    (8200, 8999),
    (9000, 11999),
    (12000, 15999),
];

const DATA: &str = "crates/poly-kernelbase/data/system-messages.tsv";
const TABLE: &str = "crates/poly-kernelbase/src/sysmsg.rs";

/// One code: its number, constant name and message text.
#[derive(Debug, PartialEq)]
struct Entry {
    code: u32,
    name: String,
    text: String,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let dir = match args {
        [] => download(root)?,
        [flag, dir] if flag == "--from" => PathBuf::from(dir),
        _ => return Err("usage: cargo xtask system-messages [--from <dir with the pages as LO-HI.html>]".into()),
    };
    let mut all = Vec::new();
    for &(lo, hi) in PAGES {
        let path = dir.join(format!("{lo}-{hi}.html"));
        let html = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let entries = parse(&html).map_err(|e| format!("{}: {e}", path.display()))?;
        if entries.len() < 100 {
            return Err(format!("{}: only {} codes; has the page's layout changed?", path.display(), entries.len()));
        }
        if let Some(e) = entries.iter().find(|e| e.code < lo || e.code > hi) {
            return Err(format!("{}: code {} outside {lo}-{hi}", path.display(), e.code));
        }
        all.extend(entries);
    }
    all.sort_by_key(|e| e.code);
    if let Some(w) = all.windows(2).find(|w| w[0].code == w[1].code) {
        return Err(format!("code {} is listed twice", w[0].code));
    }
    if let Some(e) = all.iter().find(|e| e.code > u32::from(u16::MAX)) {
        return Err(format!("code {} does not fit the table's u16 codes", e.code));
    }
    let data = root.join(DATA);
    std::fs::create_dir_all(data.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&data, tsv(&all)).map_err(|e| format!("{}: {e}", data.display()))?;
    let table = root.join(TABLE);
    std::fs::write(&table, rust(&all)).map_err(|e| format!("{}: {e}", table.display()))?;
    eprintln!("==> {} system messages: wrote {} and {}", all.len(), data.display(), table.display());
    Ok(())
}

fn download(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("target").join("learn-system-error-codes");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for &(lo, hi) in PAGES {
        let file = dir.join(format!("{lo}-{hi}.html"));
        if file.exists() {
            continue;
        }
        let url = format!("{BASE}{lo}-{hi}-");
        eprintln!("==> downloading {url}");
        let part = file.with_extension("part");
        let ok = Command::new("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&part).arg(&url).status();
        if !ok.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("downloading {url} failed (or pass --from <dir>)"));
        }
        std::fs::rename(&part, &file).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// The inner HTML of every `<p>` element, in order.
fn paragraphs(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<p>") {
        let body = &rest[start + 3..];
        let Some(end) = body.find("</p>") else { break };
        out.push(&body[..end]);
        rest = &body[end + 4..];
    }
    out
}

/// A code's first paragraph: anchors, then `<strong>NAME</strong>`.
fn header(p: &str) -> Option<&str> {
    let mut s = p.trim();
    while let Some(r) = s.strip_prefix("<span id=\"") {
        let close = r.find("</span>")?;
        s = r[close + 7..].trim_start();
    }
    let name = s.strip_prefix("<strong>")?.strip_suffix("</strong>")?;
    let ok = !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && name.bytes().next().is_some_and(|b| b.is_ascii_uppercase());
    ok.then_some(name)
}

/// `N (0xH)`, both forms of the same number.
fn number(p: &str) -> Option<u32> {
    let (dec, hex) = p.trim().split_once(" (0x")?;
    let n: u32 = dec.parse().ok()?;
    let h = u32::from_str_radix(hex.strip_suffix(')')?, 16).ok()?;
    (n == h).then_some(n)
}

/// Message text: tags dropped, entities decoded, white space collapsed.
fn text(p: &str) -> Result<String, String> {
    let mut plain = String::new();
    let mut rest = p;
    while let Some(lt) = rest.find('<') {
        plain.push_str(&rest[..lt]);
        let gt = rest[lt..].find('>').ok_or("an unclosed tag")?;
        rest = &rest[lt + gt + 1..];
    }
    plain.push_str(rest);
    let mut decoded = String::new();
    let mut rest = plain.as_str();
    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        let semi = rest[amp..].find(';').ok_or("an unterminated entity")?;
        let entity = &rest[amp + 1..amp + semi];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "nbsp" => ' ',
            e if e.starts_with("#x") => char::from_u32(u32::from_str_radix(&e[2..], 16).map_err(|x| x.to_string())?)
                .ok_or("a bad character reference")?,
            e if e.starts_with('#') => {
                char::from_u32(e[1..].parse().map_err(|_| format!("a bad character reference &{e};"))?)
                    .ok_or("a bad character reference")?
            }
            e => return Err(format!("an unknown entity &{e};")),
        };
        decoded.push(if c == '\u{a0}' { ' ' } else { c });
        rest = &rest[amp + semi + 1..];
    }
    decoded.push_str(rest);
    Ok(decoded.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Every code on one page.
fn parse(html: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut ps = paragraphs(html).into_iter().peekable();
    while let Some(p) = ps.next() {
        let Some(name) = header(p) else { continue };
        let code = ps.next().and_then(number).ok_or_else(|| format!("{name}: no `N (0xH)` paragraph"))?;
        let body = ps.next().filter(|p| header(p).is_none()).ok_or_else(|| format!("{name}: no message text"))?;
        let text = text(body).map_err(|e| format!("{name}: {e}"))?;
        if text.is_empty() {
            return Err(format!("{name}: empty message text"));
        }
        out.push(Entry { code, name: name.to_string(), text });
    }
    Ok(out)
}

fn tsv(all: &[Entry]) -> String {
    let mut s = String::new();
    for e in all {
        let _ = writeln!(s, "{}\t{}\t{}", e.code, e.name, e.text);
    }
    s
}

fn rust(all: &[Entry]) -> String {
    let mut s = String::from(
        "\
//! The system message table (English, United States), generated by
//! `cargo xtask system-messages` (ADR 0101) from Microsoft Learn's \"System
//! Error Codes\" pages (0-499 through 12000-15999); the same data is in
//! `data/system-messages.tsv`. Do not edit.
//!
//! Message `CODES[i]` is `TEXT[ENDS[i - 1]..ENDS[i]]` (from 0 for the
//! first), without the CR LF a message ends with.

#![allow(clippy::unreadable_literal)]

",
    );
    // `rustfmt::skip`: the layout is the generator's, so `ci lint`'s
    // formatting check passes on the file as written.
    let _ = writeln!(s, "/// The codes, ascending.\n#[rustfmt::skip]\npub static CODES: [u16; {}] = [", all.len());
    for chunk in all.chunks(16) {
        let line: Vec<String> = chunk.iter().map(|e| e.code.to_string()).collect();
        let _ = writeln!(s, "    {},", line.join(", "));
    }
    s.push_str("];\n\n");
    let _ = writeln!(
        s,
        "/// Where each message's text ends in [`TEXT`].\n#[rustfmt::skip]\npub static ENDS: [u32; {}] = [",
        all.len()
    );
    let mut end = 0usize;
    let ends: Vec<usize> = all
        .iter()
        .map(|e| {
            end += e.text.len();
            end
        })
        .collect();
    for chunk in ends.chunks(12) {
        let line: Vec<String> = chunk.iter().map(usize::to_string).collect();
        let _ = writeln!(s, "    {},", line.join(", "));
    }
    s.push_str("];\n\n/// The texts, one after the other.\n#[rustfmt::skip]\npub static TEXT: &str = concat!(\n");
    for e in all {
        let _ = writeln!(s, "    // {} {}\n    {:?},", e.code, e.name, e.text);
    }
    s.push_str(");\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_page() {
        let html = "<p>intro</p>\
            <p><span id=\"ERROR_A\"></span><span id=\"error_a\"></span><strong>ERROR_A</strong></p>\
            <p>2 (0x2)</p><p>The &quot;file&quot; <strong>NULL</strong> &gt; <a href=\"x\">link</a>\n  text.</p>\
            <p>This value was also named <strong>ERROR_B</strong></p>\
            <p><span id=\"ERROR_C_64k\"></span><strong>ERROR_C_64k</strong></p><p>26 (0x1A)</p><p>%1 is.</p>\
            <p><a href=\"system-error-codes\">System Error Codes</a></p><p>&nbsp;</p>";
        let e = parse(html).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], Entry { code: 2, name: "ERROR_A".into(), text: "The \"file\" NULL > link text.".into() });
        assert_eq!(e[1], Entry { code: 26, name: "ERROR_C_64k".into(), text: "%1 is.".into() });
    }

    #[test]
    fn rejects_a_broken_entry() {
        assert!(parse("<p><strong>ERROR_A</strong></p><p>2 (0x3)</p><p>x</p>").is_err());
        assert!(parse("<p><strong>ERROR_A</strong></p><p>2 (0x2)</p><p><strong>ERROR_B</strong></p>").is_err());
    }

    #[test]
    fn writes_offsets() {
        let all = [
            Entry { code: 0, name: "A".into(), text: "ab".into() },
            Entry { code: 7, name: "B".into(), text: "c\"d".into() },
        ];
        let r = rust(&all);
        assert!(r.contains("#[rustfmt::skip]\npub static CODES: [u16; 2] = [\n    0, 7,\n];"));
        assert!(r.contains("#[rustfmt::skip]\npub static ENDS: [u32; 2] = [\n    2, 5,\n];"));
        assert!(r.contains("    \"c\\\"d\",\n"));
        assert_eq!(tsv(&all), "0\tA\tab\n7\tB\tc\"d\n");
    }
}
