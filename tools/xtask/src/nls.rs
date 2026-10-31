//! `cargo xtask nls-tables [--from <dir>]`: turn the Unicode Consortium's
//! public code page mapping files into the tables `poly-nls` compiles in
//! (ADR 0094).
//!
//! The sources are the "Microsoft vendor" mapping files the Unicode
//! Consortium publishes (<https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/>,
//! `WINDOWS/CPnnn.TXT` and `PC/CPnnn.TXT`): three columns, the code page's
//! byte or double byte in hex, the Unicode code point, a comment; a lead
//! byte of a double-byte code page is a line with an empty second column
//! and the comment `#DBCS LEAD BYTE`, an unassigned byte one with
//! `#UNDEFINED`. Each file is pinned by its SHA-256 below; without
//! `--from` they are downloaded with `curl` into `target/nls-mappings/`
//! (a directory of their own, only read as data).
//!
//! What is written (checked in, so a build needs neither the files nor this
//! tool):
//!
//! - `crates/poly-nls/src/tables.rs`: every single-byte code page's bytes
//!   0x80–0xFF (the files map 0x00–0x7F to ASCII, which this tool checks),
//!   and per double-byte code page its single bytes 0x80–0xFF, its lead
//!   bytes (a row number per byte), the trail byte range of its rows, and
//!   the lead and trail byte ranges as `CPINFO` reports them.
//! - `crates/poly-nls/data/cpNNN.bin`: a double-byte code page's cells,
//!   row after row, `hi - lo + 1` little-endian `u16` code points per row
//!   (0 where the file assigns nothing).
//!
//! Code page 1252 is not generated: `poly_rtl::codepage` has it (ADR 0071)
//! and `poly-nls` uses that table.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const BASE: &str = "https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT";

/// `(code page, directory under BASE, SHA-256 of CPnnn.TXT)`.
const SOURCES: &[(u16, &str, &str)] = &[
    (437, "PC", "6bad4dabcdf5940227c7d81fab130dcb18a77850b5d79de28b5dc4e047b0aaac"),
    (737, "PC", "9bf9ffe61e03bdeecd68e806e12af562fd638285a8f0fbabc8fc9cd137d65fbf"),
    (775, "PC", "3853eeb4f9bd2d760ba616dd857646efa653549d1532e7bce509f7fda65e470b"),
    (850, "PC", "ffdcc3c1c72f1aef600a63547100ef3dc452a09ad84923d382085519751c7479"),
    (852, "PC", "440d098e9f2b79eeacbe2bbc1814960b6554c885740615047f5b528c2947afb6"),
    (855, "PC", "b77ca096db4ff4fb1323c33804271575ce15d778c8b983f28405438e73ca5925"),
    (857, "PC", "6396619b918ce2fe3c366630e5652b25153bbd4bb01c8109ba8f7ed8a261658d"),
    (860, "PC", "1b3f983eac02d9ae9fc28106f2f3476ca1e4b337c7287f1a004372e14dd11e6a"),
    (861, "PC", "51c7f3ef5a435e69637237be2a414aef8dfc4f891365b686ff8d56b681f32074"),
    (862, "PC", "054c7d796da8c57a437955948e156077f3cc1b85c8f240fc375150fea89b438a"),
    (863, "PC", "f467a2a652ce3f74bb3fa86c8767dd06cbde90edfb73bf3a5541ae4cbe806d7b"),
    (865, "PC", "e31eeb03a39a5fbdd5e23de60a22af4219c9987de2088386855ab20f273f470a"),
    (866, "PC", "abcc96dd4253321eb5e542c1ece3adab10df0cc20ec5d1124a0cec22d636c924"),
    (869, "PC", "a98207634e459cbfe895e98a5e58c4f787283191d9cbfb977fec3207a2d85203"),
    (874, "WINDOWS", "f45277e205ff5aa8f0625c52b04614bbd9d8836c142e9b90235748b0a9921ec9"),
    (932, "WINDOWS", "c9bc0b0cd42e0fbcb82a09635bb5abed86afbdd4abc9e76fa5716638217cb59f"),
    (936, "WINDOWS", "b86f601c575e9ab457380b6f7abef03c75499cc6075bdc8b4b27f3f2de74bf6a"),
    (949, "WINDOWS", "f57e6fef9d1ed44f445e319a8e1b879ce6b6064520ab571c341e47b892e32abc"),
    (950, "WINDOWS", "ed403857b05e07ecd5667c7eff6b25898cb1fefe2d06cfe718d82d631e6058b6"),
    (1250, "WINDOWS", "e6535e3c81f4aff1a4b369c46e588a2423d6a75da341e01a6cea108c7542c19a"),
    (1251, "WINDOWS", "d9d491808bd7956c26ef8ed07fa63015bfab32860720e07d5bf3a64a609927ee"),
    (1253, "WINDOWS", "85d917854926ef09b22abefd19c7b61494e229fa391b648dc332f1fac0879ce3"),
    (1254, "WINDOWS", "f93569399f81abce90441793c118d2524770b58d9c5774de3233c64d88c9fca0"),
    (1255, "WINDOWS", "67ab4e3e7c088be41e0f230fa9c2c5b9a803395bec8027a5ee77df74432290c7"),
    (1256, "WINDOWS", "fd5835c5e3be668d4fa3dc3d0d3a6491795646fc9629c0892f45dd08aa72f367"),
    (1257, "WINDOWS", "5ac72f382ea4f64fe61854e0dae7d311583bd1dd20797697d4dce32b175a268b"),
    (1258, "WINDOWS", "2457d5d0fbb1d444136c579edc488d0a34474d58a36b6eb2adacba88c068df2e"),
];

const TABLES: &str = "crates/poly-nls/src/tables.rs";
const DATA: &str = "crates/poly-nls/data";

/// One parsed mapping file.
struct Mapping {
    id: u16,
    /// Single bytes: code point, `None` unassigned.
    single: [Option<u16>; 256],
    lead: [bool; 256],
    /// Double bytes `(lead << 8 | trail, code point)`.
    double: Vec<(u16, u16)>,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let from = match args {
        [] => None,
        [flag, dir] if flag == "--from" => Some(PathBuf::from(dir)),
        _ => return Err("usage: cargo xtask nls-tables [--from <dir with CPnnn.TXT>]".into()),
    };
    let dir = match from {
        Some(d) => d,
        None => download(root)?,
    };
    let mut maps = Vec::new();
    for &(id, _, sha) in SOURCES {
        let path = dir.join(format!("CP{id}.TXT"));
        check_sha(&path, sha)?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        maps.push(parse(id, &text).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let data = root.join(DATA);
    std::fs::create_dir_all(&data).map_err(|e| format!("{}: {e}", data.display()))?;
    let mut src = String::new();
    src.push_str(HEADER);
    let mut dbcs = Vec::new();
    for m in &maps {
        if m.double.is_empty() {
            write_sbcs(&mut src, m)?;
        } else {
            dbcs.push(m);
        }
    }
    src.push_str("\n/// The single-byte code pages, by identifier.\npub static SBCS: &[(u16, &[u16; 128])] = &[\n");
    for m in maps.iter().filter(|m| m.double.is_empty()) {
        let _ = writeln!(src, "    ({0}, &CP{0}),", m.id);
    }
    src.push_str("];\n");
    for m in &dbcs {
        let (bin, lo, hi, rows) = cells(m);
        let file = data.join(format!("cp{}.bin", m.id));
        std::fs::write(&file, &bin).map_err(|e| format!("{}: {e}", file.display()))?;
        write_dbcs(&mut src, m, lo, hi, rows);
    }
    src.push_str(
        "\n/// The double-byte code pages' ranges (no cells: cheap to link).\npub static DBCS_INFO: &[&DbcsInfo] = &[",
    );
    for m in &dbcs {
        let _ = write!(src, "&CP{}_INFO, ", m.id);
    }
    src.push_str("];\n\n/// The double-byte code pages.\npub static DBCS: &[&Dbcs] = &[");
    for m in &dbcs {
        let _ = write!(src, "&CP{}, ", m.id);
    }
    src.push_str("];\n");
    let out = root.join(TABLES);
    std::fs::write(&out, src).map_err(|e| format!("{}: {e}", out.display()))?;
    eprintln!("==> wrote {} and {} data files", out.display(), dbcs.len());
    Ok(())
}

const HEADER: &str = "\
//! Code page tables, generated by `cargo xtask nls-tables` (ADR 0094) from
//! the Unicode Consortium's public mapping files
//! (<https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/>, `WINDOWS/` and
//! `PC/`, pinned by SHA-256 in `tools/xtask/src/nls.rs`). Do not edit.
//!
//! A single-byte table holds the code points of bytes 0x80-0xFF (0 where
//! the file assigns none); a double-byte code page's cells are in
//! `data/cpNNN.bin`, row by row.

#![allow(clippy::unreadable_literal)]

/// A double-byte code page's lead and trail byte ranges (inclusive).
#[derive(Debug)]
pub struct DbcsInfo {
    pub id: u16,
    pub leads: &'static [(u8, u8)],
    pub trails: &'static [(u8, u8)],
}

/// A double-byte code page.
#[derive(Debug)]
pub struct Dbcs {
    pub info: &'static DbcsInfo,
    /// Bytes 0x80-0xFF that stand alone (0: unassigned or a lead byte).
    pub single: [u16; 128],
    /// Per byte, its row number + 1 if it is a lead byte.
    pub rows: [u8; 256],
    /// The trail bytes every row covers.
    pub lo: u8,
    pub hi: u8,
    /// `rows * (hi - lo + 1)` little-endian code points.
    pub cells: &'static [u8],
}
";

fn download(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("target").join("nls-mappings");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for &(id, sub, _) in SOURCES {
        let file = dir.join(format!("CP{id}.TXT"));
        if file.exists() {
            continue;
        }
        let url = format!("{BASE}/{sub}/CP{id}.TXT");
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

fn check_sha(path: &Path, want: &str) -> Result<(), String> {
    let out = Command::new("sha256sum").arg(path).output().map_err(|e| format!("sha256sum: {e}"))?;
    let got = String::from_utf8_lossy(&out.stdout);
    if got.split_whitespace().next() != Some(want) {
        return Err(format!("{} does not have the pinned SHA-256 {want} (got {got})", path.display()));
    }
    Ok(())
}

fn hex(s: &str) -> Result<u32, String> {
    let t = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).ok_or_else(|| format!("not hex: {s}"))?;
    u32::from_str_radix(t, 16).map_err(|e| format!("{s}: {e}"))
}

fn parse(id: u16, text: &str) -> Result<Mapping, String> {
    let mut m = Mapping { id, single: [None; 256], lead: [false; 256], double: Vec::new() };
    for line in text.lines() {
        // The PC files end with a DOS end-of-file mark (Ctrl+Z).
        let line = line.trim_end().trim_matches('\u{1A}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (fields, comment) = line.split_once('#').unwrap_or((line, ""));
        let mut cols = fields.split_whitespace();
        let code = hex(cols.next().ok_or("empty line")?)?;
        match cols.next() {
            Some(u) => {
                let u = hex(u)?;
                let u = u16::try_from(u).map_err(|_| format!("{u:#x} is past the BMP"))?;
                if code < 0x100 {
                    m.single[code as usize] = Some(u);
                } else {
                    m.double.push((code as u16, u));
                }
            }
            None if comment.contains("DBCS LEAD BYTE") => m.lead[code as usize] = true,
            None => {} // UNDEFINED
        }
    }
    for b in 0..0x80 {
        if m.single[b] != Some(b as u16) {
            return Err(format!("byte {b:#x} is not ASCII"));
        }
    }
    for &(code, _) in &m.double {
        if !m.lead[usize::from(code >> 8)] {
            return Err(format!("{code:#x} has no lead byte"));
        }
    }
    Ok(m)
}

fn write_sbcs(src: &mut String, m: &Mapping) -> Result<(), String> {
    if m.lead.iter().any(|&l| l) {
        return Err(format!("CP{} has lead bytes but no double bytes", m.id));
    }
    let _ = writeln!(src, "\n/// Code page {}, bytes 0x80-0xFF.", m.id);
    let _ = write!(src, "pub static CP{}: [u16; 128] = [", m.id);
    write_u16s(src, &m.single[0x80..]);
    src.push_str("];\n");
    Ok(())
}

fn write_u16s(src: &mut String, v: &[Option<u16>]) {
    for (i, c) in v.iter().enumerate() {
        if i % 12 == 0 {
            src.push_str("\n    ");
        } else {
            src.push(' ');
        }
        let _ = write!(src, "0x{:04X},", c.unwrap_or(0));
    }
    src.push('\n');
}

fn ranges(set: impl Iterator<Item = u8>) -> Vec<(u8, u8)> {
    let mut out: Vec<(u8, u8)> = Vec::new();
    for b in set {
        match out.last_mut() {
            Some((_, hi)) if u16::from(*hi) + 1 == u16::from(b) => *hi = b,
            _ => out.push((b, b)),
        }
    }
    out
}

/// The cells of a double-byte code page: `(bytes, lo, hi, row of each lead)`.
fn cells(m: &Mapping) -> (Vec<u8>, u8, u8, [u8; 256]) {
    let lo = m.double.iter().map(|&(c, _)| c as u8).min().unwrap_or(0x40);
    let hi = m.double.iter().map(|&(c, _)| c as u8).max().unwrap_or(0xFE);
    let width = usize::from(hi - lo) + 1;
    let mut rows = [0u8; 256];
    let mut n = 0u8;
    for (row, &lead) in rows.iter_mut().zip(m.lead.iter()) {
        if lead {
            n += 1;
            *row = n;
        }
    }
    let mut cells = vec![0u16; usize::from(n) * width];
    for &(code, u) in &m.double {
        let row = usize::from(rows[usize::from(code >> 8)] - 1);
        cells[row * width + usize::from(code as u8 - lo)] = u;
    }
    let bin = cells.iter().flat_map(|c| c.to_le_bytes()).collect();
    (bin, lo, hi, rows)
}

fn write_dbcs(src: &mut String, m: &Mapping, lo: u8, hi: u8, rows: [u8; 256]) {
    let leads = ranges((0..=255u8).filter(|&b| m.lead[usize::from(b)]));
    let mut trail = [false; 256];
    for &(c, _) in &m.double {
        trail[usize::from(c as u8)] = true;
    }
    let trails = ranges((0..=255u8).filter(|&b| trail[usize::from(b)]));
    let id = m.id;
    let _ = writeln!(src, "\n/// Code page {id}'s lead and trail bytes.");
    let _ = writeln!(
        src,
        "pub static CP{id}_INFO: DbcsInfo = DbcsInfo {{ id: {id}, leads: &{leads:?}, trails: &{trails:?} }};"
    );
    let _ = writeln!(src, "\n/// Code page {id}.");
    let _ = writeln!(src, "pub static CP{id}: Dbcs = Dbcs {{");
    let _ = writeln!(src, "    info: &CP{id}_INFO,");
    let mut single = [None; 128];
    for b in 0x80..0x100 {
        if !m.lead[b] {
            single[b - 0x80] = m.single[b];
        }
    }
    src.push_str("    single: [");
    write_u16s(src, &single);
    src.push_str("    ],\n    rows: [");
    for (i, r) in rows.iter().enumerate() {
        if i % 16 == 0 {
            src.push_str("\n        ");
        } else {
            src.push(' ');
        }
        let _ = write!(src, "{r},");
    }
    let _ = writeln!(src, "\n    ],\n    lo: 0x{lo:02X},\n    hi: 0x{hi:02X},");
    let _ = writeln!(src, "    cells: include_bytes!(\"../data/cp{id}.bin\"),\n}};");
}
