//! `cargo xtask win imports <exe>`: what a Windows program imports, and what
//! of it Poly lacks (ADR 0087).
//!
//! Every import of the executable — the import directory and the
//! delay-load directory, by name or by ordinal (PE format specification,
//! "The .idata Section", "Delay-Load Import Tables") — is followed the way
//! our `ntdll`'s loader would bind it: an API-set name goes to the DLL
//! that hosts the contract (`poly_ldr::apiset`, ADR 0071), a forwarder of
//! one of our DLLs to its target, until code is found or the chain breaks.
//! Each import gets a [`Verdict`]:
//!
//! - **present**: one of our DLLs exports it as code; the coverage map's
//!   status of that export says how far it is (`claimed`, `green`);
//! - **stub**: present, but the coverage map marks it `stub` (it exists and
//!   fails plausibly, ADR 0055's "Status" vocabulary);
//! - **missing function**: the DLL is ours but does not export the name
//!   (and whether the coverage map knows the name for that DLL at all);
//! - **missing DLL**: no DLL of ours (or an API set Poly does not host).
//!
//! "Our DLLs" are the ones the kernel installs in `System32` — `ntdll.dll`
//! and the Win32 modules of [`crate::win32::OUTPUTS`] — read from the same
//! content-hash caches the kernel embeds them from, so the report is about
//! the current sources. Nothing here reads anything but export and import
//! *names* (CLEAN_ROOM.md).

use crate::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// What an import names.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sym {
    Name(String),
    Ordinal(u16),
}

impl std::fmt::Display for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sym::Name(n) => f.write_str(n),
            Sym::Ordinal(o) => write!(f, "#{o}"),
        }
    }
}

/// One import of an executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// The DLL as the executable names it.
    pub dll: String,
    pub sym: Sym,
    /// From the delay-load directory: bound on first call, not at start.
    pub delay: bool,
}

/// Where one export of ours leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Code,
    /// A forwarder: `dll` as the string names it (no `.dll`), and the name.
    Forward {
        dll: String,
        name: String,
    },
    /// A forwarder to an ordinal, which the report does not follow.
    ForwardOrdinal(String),
}

/// The export tables of our DLLs, by lower-case file name: the names, and
/// the names of the exports with an ordinal programs import (`oleaut32`).
#[derive(Clone, Debug, Default)]
pub struct Ours {
    pub dlls: BTreeMap<String, BTreeMap<String, Target>>,
    pub ordinals: BTreeMap<String, BTreeMap<u16, String>>,
}

/// The coverage map's statuses of the exports of each DLL (lower-case file
/// name → symbol → status; a symbol without a status is `missing`).
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub dlls: BTreeMap<String, BTreeMap<String, String>>,
}

impl Coverage {
    fn status(&self, dll: &str, name: &str) -> Option<&str> {
        self.dlls.get(dll)?.get(name).map(String::as_str)
    }
}

/// What Poly has of one import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Code in `host` (after API sets and forwarders); its coverage status.
    Present { host: String, status: String },
    /// Code in `host` that the coverage map marks `stub`.
    Stub { host: String },
    /// `host` is ours but lacks the name; `known`: the coverage map lists
    /// it for `host` (so Windows has it there).
    MissingFunction { host: String, name: Sym, known: bool },
    /// Not one of our DLLs (`host`: the file it resolves to, or the API
    /// set Poly does not host).
    MissingDll { host: String },
}

/// Forwarder chains longer than this are reported as missing (the loader
/// has a limit of its own).
const MAX_HOPS: usize = 8;

/// A module name as a file name: lower case, `.dll` added when it has no
/// extension (a forwarder string names its DLL without one).
fn file_name(name: &str) -> String {
    let low = name.to_ascii_lowercase();
    if low.contains('.') { low } else { format!("{low}.dll") }
}

/// The DLL a module name loads: the API-set host, or the name itself.
fn host_of(name: &str) -> std::result::Result<String, String> {
    use poly_ldr::apiset::{ApiSet, resolve};
    let wide: Vec<u16> = name.encode_utf16().collect();
    match resolve(&wide) {
        ApiSet::NotApiSet => Ok(file_name(name)),
        ApiSet::Host(h) => Ok(h.to_string()),
        ApiSet::Absent => Err(format!("{} (API set not hosted)", name.to_ascii_lowercase())),
    }
}

/// Follow one import to its code (see the module docs).
pub fn classify(dll: &str, sym: &Sym, ours: &Ours, cov: &Coverage) -> Verdict {
    let mut host = match host_of(dll) {
        Ok(h) => h,
        Err(h) => return Verdict::MissingDll { host: h },
    };
    let mut name = match sym {
        Sym::Name(n) => n.clone(),
        // An ordinal binds to the export at that ordinal: one with a name
        // is followed as that name.
        Sym::Ordinal(o) => {
            if !ours.dlls.contains_key(&host) {
                return Verdict::MissingDll { host };
            }
            match ours.ordinals.get(&host).and_then(|m| m.get(o)) {
                Some(n) => n.clone(),
                None => return Verdict::MissingFunction { host, name: sym.clone(), known: false },
            }
        }
    };
    // The imported DLL's own entry: its status stands for code whose host
    // has no entry of that name (a function Windows keeps in `kernel32`
    // that Poly's API-set table sends to `kernelbase`).
    let first = cov.status(&host, &name).map(str::to_string);
    for _ in 0..MAX_HOPS {
        let Some(exports) = ours.dlls.get(&host) else { return Verdict::MissingDll { host } };
        match exports.get(&name) {
            None => {
                let known = cov.status(&host, &name).is_some();
                return Verdict::MissingFunction { host, name: Sym::Name(name), known };
            }
            Some(Target::Code) => {
                let status = cov.status(&host, &name).or(first.as_deref()).unwrap_or("unlisted").to_string();
                return if status == "stub" { Verdict::Stub { host } } else { Verdict::Present { host, status } };
            }
            Some(Target::ForwardOrdinal(raw)) => {
                return Verdict::MissingFunction { host, name: Sym::Name(format!("{name} -> {raw}")), known: true };
            }
            Some(Target::Forward { dll, name: to }) => {
                host = match host_of(dll) {
                    Ok(h) => h,
                    Err(h) => return Verdict::MissingDll { host: h },
                };
                name = to.clone();
            }
        }
    }
    Verdict::MissingFunction { host, name: Sym::Name(name), known: false }
}

/// The imports of a PE image, in directory order, then the delay-load ones.
pub fn imports_of(bytes: &[u8]) -> Result<Vec<Import>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    let mut out = Vec::new();
    let sym = |s: poly_pe::ImportSymbol<'_>| match s {
        poly_pe::ImportSymbol::Name { name, .. } => Sym::Name(String::from_utf8_lossy(name).into_owned()),
        poly_pe::ImportSymbol::Ordinal(o) => Sym::Ordinal(o),
    };
    let err = |e: poly_pe::Error| format!("imports: {e:?}");
    if let Some(dirs) = pe.imports().map_err(err)? {
        for d in dirs {
            let d = d.map_err(err)?;
            let dll = String::from_utf8_lossy(d.dll_name().map_err(err)?).into_owned();
            for e in d.entries().map_err(err)? {
                out.push(Import { dll: dll.clone(), sym: sym(e.map_err(err)?.symbol), delay: false });
            }
        }
    }
    if let Some(dirs) = pe.delay_imports().map_err(err)? {
        for d in dirs {
            let d = d.map_err(err)?;
            let dll = String::from_utf8_lossy(d.dll_name().map_err(err)?).into_owned();
            for e in d.entries() {
                out.push(Import { dll: dll.clone(), sym: sym(e.map_err(err)?.symbol), delay: true });
            }
        }
    }
    Ok(out)
}

/// The ordinals of one of our DLLs' named exports.
pub fn ordinals_of(bytes: &[u8]) -> Result<BTreeMap<u16, String>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    let mut out = BTreeMap::new();
    let Some(exports) = pe.exports().map_err(|e| format!("exports: {e:?}"))? else { return Ok(out) };
    for e in exports.names() {
        let (name, _) = e.map_err(|e| format!("exports: {e:?}"))?;
        let ordinal = exports.ordinal_of(name).map_err(|e| format!("exports: {e:?}"))?;
        if let Some(o) = ordinal.and_then(|o| u16::try_from(o).ok()) {
            out.insert(o, String::from_utf8_lossy(name).into_owned());
        }
    }
    Ok(out)
}

/// The export table of one of our DLLs.
pub fn exports_of(bytes: &[u8]) -> Result<BTreeMap<String, Target>> {
    use poly_pe::{ExportTarget, ForwardTarget};
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    let mut out = BTreeMap::new();
    let Some(exports) = pe.exports().map_err(|e| format!("exports: {e:?}"))? else { return Ok(out) };
    for e in exports.names() {
        let (name, _) = e.map_err(|e| format!("exports: {e:?}"))?;
        let target = match exports.by_name(name).map_err(|e| format!("exports: {e:?}"))? {
            Some(ExportTarget::Rva(_)) => Target::Code,
            Some(ExportTarget::Forwarder(f)) => match f.target {
                ForwardTarget::Name(n) => Target::Forward {
                    dll: String::from_utf8_lossy(f.dll).into_owned(),
                    name: String::from_utf8_lossy(n).into_owned(),
                },
                ForwardTarget::Ordinal(_) => Target::ForwardOrdinal(String::from_utf8_lossy(f.raw).into_owned()),
            },
            None => continue,
        };
        out.insert(String::from_utf8_lossy(name).into_owned(), target);
    }
    Ok(out)
}

/// The statuses of `coverage/windows/api/<dll>.json` (one entry object
/// per symbol; see `tools/coverage`).
pub fn coverage_of(text: &str) -> Result<BTreeMap<String, String>> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    for e in entries {
        let Some(symbol) = e.get("symbol").and_then(|s| s.as_str()) else { continue };
        let status = e.get("status").and_then(|s| s.as_str()).unwrap_or("missing");
        out.insert(symbol.to_string(), status.to_string());
    }
    Ok(out)
}

/// One import and its verdict.
#[derive(Clone, Debug)]
pub struct Row {
    pub import: Import,
    pub verdict: Verdict,
}

/// The whole report.
#[derive(Clone, Debug)]
pub struct Report {
    pub exe: String,
    pub rows: Vec<Row>,
}

impl Report {
    pub fn new(exe: &str, imports: Vec<Import>, ours: &Ours, cov: &Coverage) -> Report {
        let rows = imports
            .into_iter()
            .map(|import| {
                let verdict = classify(&import.dll, &import.sym, ours, cov);
                Row { import, verdict }
            })
            .collect();
        Report { exe: exe.to_string(), rows }
    }

    /// Counts: (imports, present, stub, missing functions, missing-DLL imports).
    pub fn counts(&self) -> (usize, usize, usize, usize, usize) {
        let mut c = (self.rows.len(), 0, 0, 0, 0);
        for r in &self.rows {
            match r.verdict {
                Verdict::Present { .. } => c.1 += 1,
                Verdict::Stub { .. } => c.2 += 1,
                Verdict::MissingFunction { .. } => c.3 += 1,
                Verdict::MissingDll { .. } => c.4 += 1,
            }
        }
        c
    }

    /// Whether every import binds (no missing function or DLL): the loader
    /// would start the program.
    pub fn binds(&self) -> bool {
        let (_, _, _, f, d) = self.counts();
        f == 0 && d == 0
    }

    /// The report as Markdown (the generated part of `docs/windows/*-gaps.md`).
    pub fn markdown(&self) -> String {
        let mut s = String::new();
        let (all, present, stub, missing, dll) = self.counts();
        let _ = writeln!(s, "Imports of `{}`: {all}.\n", self.exe);
        let _ = writeln!(s, "| Verdict | Imports |\n|---|---:|");
        let _ = writeln!(s, "| present (code in one of our DLLs) | {present} |");
        let _ = writeln!(s, "| present but a stub (coverage status `stub`) | {stub} |");
        let _ = writeln!(s, "| missing function (our DLL lacks it) | {missing} |");
        let _ = writeln!(s, "| missing DLL | {dll} |\n");

        // Per imported DLL.
        let mut per: BTreeMap<String, (usize, usize, usize, usize, bool)> = BTreeMap::new();
        for r in &self.rows {
            let e = per.entry(r.import.dll.to_ascii_lowercase()).or_default();
            e.4 |= r.import.delay;
            match r.verdict {
                Verdict::Present { .. } => e.0 += 1,
                Verdict::Stub { .. } => e.1 += 1,
                Verdict::MissingFunction { .. } => e.2 += 1,
                Verdict::MissingDll { .. } => e.3 += 1,
            }
        }
        let _ = writeln!(s, "### By imported DLL\n");
        let _ = writeln!(
            s,
            "| DLL | Resolves to | Present | Stub | Missing | DLL missing |\n|---|---|---:|---:|---:|---:|"
        );
        for (dll, (p, st, m, d, delay)) in &per {
            let host = host_of(dll).unwrap_or_else(|h| h);
            let delay = if *delay { " (delay-load)" } else { "" };
            let _ = writeln!(s, "| `{dll}`{delay} | `{host}` | {p} | {st} | {m} | {d} |");
        }

        let section = |s: &mut String, title: &str, pick: &dyn Fn(&Verdict) -> Option<String>| {
            let rows: Vec<(String, String)> = self
                .rows
                .iter()
                .filter_map(|r| {
                    let note = pick(&r.verdict)?;
                    let delay = if r.import.delay { " (delay-load)" } else { "" };
                    Some((format!("`{}!{}`{delay}", r.import.dll.to_ascii_lowercase(), r.import.sym), note))
                })
                .collect();
            let _ = writeln!(s, "\n### {title} ({})\n", rows.len());
            if rows.is_empty() {
                let _ = writeln!(s, "None.");
                return;
            }
            let _ = writeln!(s, "| Import | Note |\n|---|---|");
            for (i, n) in rows {
                let _ = writeln!(s, "| {i} | {n} |");
            }
        };
        section(&mut s, "Missing DLLs", &|v| match v {
            Verdict::MissingDll { host } => Some(format!("no `{host}`")),
            _ => None,
        });
        section(&mut s, "Missing functions", &|v| match v {
            Verdict::MissingFunction { host, name, known } => Some(format!(
                "`{host}` lacks `{name}`{}",
                if *known { "" } else { " (not in the coverage map's list for this DLL either)" }
            )),
            _ => None,
        });
        section(&mut s, "Stubs", &|v| match v {
            Verdict::Stub { host } => Some(format!("in `{host}`")),
            _ => None,
        });
        section(&mut s, "Present", &|v| match v {
            Verdict::Present { host, status } => Some(format!("`{host}`, {status}")),
            _ => None,
        });
        s
    }
}

/// Read our DLLs from the caches the kernel embeds them from.
pub fn load_ours() -> Result<Ours> {
    let mut ours = Ours::default();
    let ntdll = crate::ntdll::ensure()?;
    let bytes = std::fs::read(&ntdll).map_err(|e| format!("{}: {e}", ntdll.display()))?;
    ours.dlls.insert("ntdll.dll".into(), exports_of(&bytes)?);
    let dir = crate::win32::ensure()?;
    for f in crate::win32::OUTPUTS.iter().filter(|f| f.ends_with(".dll")) {
        let path = dir.join(f);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        ours.dlls.insert(f.to_string(), exports_of(&bytes)?);
        ours.ordinals.insert(f.to_string(), ordinals_of(&bytes)?);
    }
    Ok(ours)
}

/// The coverage statuses of every DLL in `ours` and every DLL `imports`
/// name (resolved), from `coverage/windows/api/`.
pub fn load_coverage(root: &Path, ours: &Ours, imports: &[Import]) -> Result<Coverage> {
    let mut cov = Coverage::default();
    let mut names: Vec<String> = ours.dlls.keys().cloned().collect();
    names.extend(imports.iter().filter_map(|i| host_of(&i.dll).ok()));
    names.sort();
    names.dedup();
    for dll in names {
        let path = root.join("coverage/windows/api").join(format!("{dll}.json"));
        if let Ok(text) = std::fs::read_to_string(&path) {
            cov.dlls.insert(dll, coverage_of(&text).map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    Ok(cov)
}

/// `cargo xtask win imports <exe | 7zip> [--markdown <file>] [--check]`:
/// print the report (`--markdown`: also write it there); `--check`: fail
/// unless every import binds.
pub fn command(args: &[String]) -> Result {
    let mut exe = None;
    let mut markdown = None;
    let mut check = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--markdown" => markdown = Some(it.next().ok_or("--markdown needs a file")?.clone()),
            "--check" => check = true,
            flag if flag.starts_with("--") => return Err(format!("win imports: unknown option {flag}")),
            path => exe = Some(path.to_string()),
        }
    }
    let exe = exe.ok_or("usage: cargo xtask win imports <exe | 7zip> [--markdown <file>] [--check]")?;
    let (path, label) = if exe == "7zip" {
        let p = crate::sevenzip::cached().ok_or("7za.exe is not in the cache: run `cargo xtask win fetch-7zip`")?;
        (p, format!("7za.exe (7-Zip Extra {}, x64)", crate::sevenzip::VERSION))
    } else {
        let p = std::path::PathBuf::from(&exe);
        let label = p.file_name().map_or(exe.clone(), |n| n.to_string_lossy().into_owned());
        (p, label)
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let imports = imports_of(&bytes)?;
    let ours = load_ours()?;
    let cov = load_coverage(&crate::root(), &ours, &imports)?;
    let report = Report::new(&label, imports, &ours, &cov);
    let text = report.markdown();
    println!("{text}");
    if let Some(out) = markdown {
        let doc = format!(
            "# Import gaps: {label}\n\n\
             Generated by `cargo xtask win imports {exe} --markdown {out}` (ADR 0087); \
             do not edit by hand, run it again.\n\n{text}"
        );
        std::fs::write(&out, doc).map_err(|e| format!("{out}: {e}"))?;
        eprintln!("==> wrote {out}");
    }
    if check && !report.binds() {
        let (_, _, _, f, d) = report.counts();
        return Err(format!("{f} imported functions and {d} imports of missing DLLs do not bind"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> Ours {
        let mut o = Ours::default();
        let mut k32 = BTreeMap::new();
        k32.insert(
            "CreateFileW".into(),
            Target::Forward { dll: "api-ms-win-core-file-l1-1-0".into(), name: "CreateFileW".into() },
        );
        k32.insert("GlobalAlloc".into(), Target::Code);
        k32.insert("HeapAlloc".into(), Target::Forward { dll: "NTDLL".into(), name: "RtlAllocateHeap".into() });
        k32.insert("Loop".into(), Target::Forward { dll: "kernel32".into(), name: "Loop".into() });
        o.dlls.insert("kernel32.dll".into(), k32);
        let mut kb = BTreeMap::new();
        kb.insert("CreateFileW".into(), Target::Code);
        kb.insert("Beep".into(), Target::Code);
        o.dlls.insert("kernelbase.dll".into(), kb);
        let mut nt = BTreeMap::new();
        nt.insert("RtlAllocateHeap".into(), Target::Code);
        o.dlls.insert("ntdll.dll".into(), nt);
        let mut ole = BTreeMap::new();
        ole.insert("SysFreeString".into(), Target::Code);
        o.dlls.insert("oleaut32.dll".into(), ole);
        o.ordinals.insert("oleaut32.dll".into(), [(6, "SysFreeString".to_string())].into());
        o
    }

    fn cov() -> Coverage {
        let mut c = Coverage::default();
        c.dlls.insert(
            "kernelbase.dll".into(),
            coverage_of(r#"[{"symbol":"CreateFileW","status":"claimed"},{"symbol":"Beep","status":"stub"},{"symbol":"GetFoo"}]"#)
                .unwrap(),
        );
        c
    }

    fn name(s: &str) -> Sym {
        Sym::Name(s.into())
    }

    #[test]
    fn follows_forwarders_and_api_sets_to_code() {
        let (o, c) = (ours(), cov());
        let present = |host: &str, status: &str| Verdict::Present { host: host.into(), status: status.into() };
        assert_eq!(classify("KERNEL32.dll", &name("CreateFileW"), &o, &c), present("kernelbase.dll", "claimed"));
        assert_eq!(
            classify("api-ms-win-core-file-l1-2-4.dll", &name("CreateFileW"), &o, &c),
            present("kernelbase.dll", "claimed")
        );
        assert_eq!(classify("kernel32.dll", &name("HeapAlloc"), &o, &c), present("ntdll.dll", "unlisted"));
        assert_eq!(classify("kernel32.dll", &name("GlobalAlloc"), &o, &c), present("kernel32.dll", "unlisted"));
        assert_eq!(classify("kernelbase.dll", &name("Beep"), &o, &c), Verdict::Stub { host: "kernelbase.dll".into() });
    }

    #[test]
    fn reports_what_is_missing() {
        let (o, c) = (ours(), cov());
        assert_eq!(
            classify("api-ms-win-core-misc-l1-1-0.dll", &name("GetFoo"), &o, &c),
            Verdict::MissingFunction { host: "kernelbase.dll".into(), name: name("GetFoo"), known: true }
        );
        assert_eq!(
            classify("kernelbase.dll", &name("Nope"), &o, &c),
            Verdict::MissingFunction { host: "kernelbase.dll".into(), name: name("Nope"), known: false }
        );
        assert_eq!(classify("USER32.dll", &Sym::Ordinal(2), &o, &c), Verdict::MissingDll { host: "user32.dll".into() });
        assert_eq!(
            classify("OLEAUT32.dll", &Sym::Ordinal(6), &o, &c),
            Verdict::Present { host: "oleaut32.dll".into(), status: "unlisted".into() }
        );
        assert_eq!(
            classify("OLEAUT32.dll", &Sym::Ordinal(2), &o, &c),
            Verdict::MissingFunction { host: "oleaut32.dll".into(), name: Sym::Ordinal(2), known: false }
        );
        assert_eq!(
            classify("kernel32.dll", &Sym::Ordinal(2), &o, &c),
            Verdict::MissingFunction { host: "kernel32.dll".into(), name: Sym::Ordinal(2), known: false }
        );
        assert!(matches!(classify("ext-ms-win-foo-l1-1-0.dll", &name("X"), &o, &c), Verdict::MissingDll { .. }));
        // A forwarding loop ends.
        assert!(matches!(classify("kernel32.dll", &name("Loop"), &o, &c), Verdict::MissingFunction { .. }));
    }

    #[test]
    fn report_counts_and_markdown() {
        let (o, c) = (ours(), cov());
        let imports = vec![
            Import { dll: "KERNEL32.dll".into(), sym: name("CreateFileW"), delay: false },
            Import { dll: "KERNEL32.dll".into(), sym: name("Nope"), delay: false },
            Import { dll: "USER32.dll".into(), sym: Sym::Ordinal(6), delay: true },
            Import { dll: "kernelbase.dll".into(), sym: name("Beep"), delay: false },
        ];
        let r = Report::new("x.exe", imports, &o, &c);
        assert_eq!(r.counts(), (4, 1, 1, 1, 1));
        assert!(!r.binds());
        let md = r.markdown();
        assert!(md.contains("| `user32.dll` (delay-load) | `user32.dll` | 0 | 0 | 0 | 1 |"), "{md}");
        let note = "`kernel32.dll` lacks `Nope` (not in the coverage map's list for this DLL either)";
        assert!(md.contains(&format!("| `kernel32.dll!Nope` | {note} |")), "{md}");
        assert!(md.contains("### Stubs (1)"));
    }

    /// The parser reads what `poly-pe` reads: one of our own test programs'
    /// import table, built into a minimal image here, would need the Win32
    /// build; instead the error path for a non-PE.
    #[test]
    fn rejects_non_pe() {
        assert!(imports_of(b"not a pe").is_err());
        assert!(exports_of(b"MZ").is_err());
    }
}
