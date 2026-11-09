//! The Windows API tracer (roadmap Phase 5; ADR 0090): record the calls a
//! program makes into the DLLs it imports, the same way on real Windows
//! and on Poly, and compare two recordings.
//!
//! `cargo xtask win trace-kit <exe> --out <dir>` writes a self-contained
//! directory: a copy of the program whose import directory names
//! `pt_<dll>.dll` instead of each `<dll>.dll` (only the name strings
//! change: a section with the new names is appended and the descriptors
//! point at it), one proxy DLL per imported DLL, `polytrace.dll` (the
//! logger, `tools/wintrace/polytrace`) and `polytrace-kit.json`. Run the
//! copy in that directory and it writes `polytrace.jsonl` (see the logger's
//! documentation for the format).
//!
//! A proxy exports exactly what the program imports from the real DLL,
//! by the same names and ordinals. A function export is a thunk
//! (`tools/wintrace/proxy/common.rs`) that calls the logger, then the real
//! function through the proxy's own import of it (the loader binds it,
//! as it binds the program's), then the logger again. Data exports (the C
//! runtime's `_iob`, `_fmode` …) are PE forwarders to the real DLL, so the
//! program gets the real object's address. The proxies are Rust `no_std`
//! cdylibs generated into `target/wintrace/build` and built like the
//! Windows test programs (ADR 0075): `-Zbuild-std` from `rust-src`,
//! `rust-lld`, offline.
//!
//! What each function's arguments are (how many, and which are integers,
//! pointers, handles or strings) comes from [`SIGNATURES`], written from
//! Microsoft Learn's prototypes; a function without an entry is logged with
//! four arguments of unknown kind. [`NORETURN`], [`TAIL`] and [`FLUSH`] say
//! which functions do not return, must see their caller's frame, or may end
//! the process.
//!
//! `cargo xtask win trace-diff <a.jsonl> <b.jsonl>` aligns two recordings
//! thread by thread in call order and reports where they part: a call only
//! one side made, or the same call with another result, last error or
//! integer or string argument (pointers and handles are compared only as
//! NULL, `INVALID_HANDLE_VALUE` or "some value").
//!
//! Only export and import *names* of the program are read (CLEAN_ROOM.md);
//! the tracer observes calls, it does not look inside any DLL.

use crate::Result;
use crate::winimports::{Import, Sym};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

mod compare;
mod kit;
mod pe;
mod proxy;
mod tables;

pub use compare::*;
pub use kit::*;
pub use pe::*;
pub use proxy::*;
pub use tables::*;

// ----- the command ------------------------------------------------------------------------

const USAGE: &str = "usage: cargo xtask win trace-kit <exe | 7zip> [<more exe>...] [--out <dir>] [--data <name>]... [--agg] [--skip <dll>]...\n       \
                     cargo xtask win trace-diff <a.jsonl> <b.jsonl> [--map-a <from>=<to>] [--map-b <from>=<to>] [--limit <n>] [--check]";

fn mapping(s: &str) -> Result<(String, String)> {
    s.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| format!("{s:?}: expected <from>=<to>"))
}

/// `cargo xtask win trace-kit ...`.
pub fn trace_kit_command(args: &[String]) -> Result {
    let mut exe = None;
    let mut extra: Vec<PathBuf> = Vec::new();
    let mut out = None;
    let mut data = Vec::new();
    let mut skip = Vec::new();
    let mut agg = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--data" => data.push(it.next().ok_or(USAGE)?.clone()),
            "--skip" => skip.push(it.next().ok_or(USAGE)?.clone()),
            "--agg" => agg = true,
            s if exe.is_none() && !s.starts_with("--") => exe = Some(s.to_string()),
            s if !s.starts_with("--") => extra.push(PathBuf::from(s)),
            _ => return Err(USAGE.into()),
        }
    }
    if agg || !extra.is_empty() || !skip.is_empty() {
        let first = PathBuf::from(exe.ok_or(USAGE)?);
        let exes: Vec<PathBuf> = std::iter::once(first).chain(extra).collect();
        let out = out.unwrap_or_else(|| work_dir().join("kit"));
        let kit = build_kit_multi(&exes, &out, &data, agg, false, &skip)?;
        for p in &kit.proxies {
            eprintln!(
                "    {} for {}: {} functions, {} forwarded variables",
                p.file,
                p.real,
                p.funcs.len(),
                p.data.len()
            );
        }
        eprintln!("==> trace kit for {} in {}", kit.exe, kit.dir.display());
        return Ok(());
    }
    let exe = match exe.as_deref() {
        Some("7zip") => {
            crate::sevenzip::cached().ok_or("7za.exe is not in the cache; run `cargo xtask win fetch-7zip`")?
        }
        Some(p) => PathBuf::from(p),
        None => return Err(USAGE.into()),
    };
    let out = out.unwrap_or_else(|| work_dir().join("kit"));
    let kit = build_kit(&exe, &out, &data)?;
    for p in &kit.proxies {
        let unknown: Vec<String> = p.funcs.iter().filter(|f| f.sig.is_empty()).map(|f| f.sym.to_string()).collect();
        eprintln!(
            "    {} for {}: {} functions, {} forwarded variables{}",
            p.file,
            p.real,
            p.funcs.len(),
            p.data.len(),
            if unknown.is_empty() {
                String::new()
            } else {
                format!("; no signature (4 arguments logged): {}", unknown.join(", "))
            }
        );
    }
    eprintln!("==> trace kit for {} in {}", kit.exe, kit.dir.display());
    Ok(())
}

/// `cargo xtask win trace-diff ...`.
pub fn trace_diff_command(args: &[String]) -> Result {
    let (mut files, mut map_a, mut map_b, mut limit, mut check) = (Vec::new(), Vec::new(), Vec::new(), 10, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--map-a" => map_a.push(mapping(it.next().ok_or(USAGE)?)?),
            "--map-b" => map_b.push(mapping(it.next().ok_or(USAGE)?)?),
            "--limit" => limit = it.next().ok_or(USAGE)?.parse().map_err(|_| USAGE)?,
            "--check" => check = true,
            s if !s.starts_with("--") => files.push(s.to_string()),
            _ => return Err(USAGE.into()),
        }
    }
    let [fa, fb] = files.as_slice() else { return Err(USAGE.into()) };
    let read = |f: &str| -> Result<Vec<Call>> {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"))?;
        parse_trace(&text).map_err(|e| format!("{f}: {e}"))
    };
    let d = diff_traces(&normalise(&read(fa)?), &normalise(&read(fb)?), &map_a, &map_b);
    print!("{}", report(&d, fa, fb, limit));
    if check && !d.divergences.is_empty() {
        return Err(format!("the traces differ ({} divergences)", d.divergences.len()));
    }
    Ok(())
}

// ----- the boot test ----------------------------------------------------------------------

/// The boot test `wintrace`: the traced 7-Zip scenario
/// (`sevenzip_trace.rs`, ADR 0110), which runs ADR 0088's steps under the
/// tracer and compares each step's trace with the committed Windows
/// reference (required: `TRACE_REQUIRED`, ADR 0114). It replaced ADR 0090's
/// three single runs.
pub fn boot_test(kernel: &Path, _syms: &std::sync::Arc<crate::symbols::Symbols>) -> Result {
    crate::sevenzip_trace::boot_test(kernel)
}

#[cfg(test)]
mod tests;
