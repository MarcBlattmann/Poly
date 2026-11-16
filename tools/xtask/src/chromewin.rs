//! Chrome for Windows on Poly, sandbox on (ADR 0155; the Windows
//! counterpart of `chromium.rs`, ADR 0130).
//!
//! The program is **Chrome for Testing**, the stable channel's win64 build
//! ([`VERSION`]), from Google's public bucket. Like 7-Zip (ADR 0087) it is
//! never committed or redistributed: `cargo xtask chrome-win fetch`
//! downloads it over HTTPS into `<main checkout>/target/chrome-win/<version>/`
//! (shared by worktrees), checks its size and SHA-256 ([`SIZE`], [`SHA256`]),
//! and unpacks it with the pure-Rust ZIP reader (`unzip.rs`) so the same
//! command works on Windows and Linux hosts (ADR 0124). Google publishes no
//! hash for the archive; the SHA-256 here is of the bytes the first
//! download gave (HTTPS from `storage.googleapis.com`), and every later
//! download must match it.
//!
//! Nothing of Chrome is read to learn how Windows behaves (CLEAN_ROOM.md):
//! it runs on Poly as a black box, and its PE import tables, names only,
//! say what it needs (`cargo xtask chrome-win imports`).
//!
//! - `cargo xtask chrome-win fetch`: download, verify, unpack.
//! - `cargo xtask chrome-win imports [--markdown FILE]`: every PE file of
//!   the package against our DLLs (and its own), rolled up.
//! - `cargo xtask chrome-win disk`: the Poly disk with Chrome under
//!   `C:\Program Files\Chrome` ([`disk`]).
//! - `cargo xtask chrome-win run [--stage N] [--trace]`: boot a stage
//!   ([`run`]).

use crate::Result;
use crate::winimports::{Import, Ours, Report, Verdict};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Chrome for Testing, stable channel.
pub const VERSION: &str = "157.0.8103.38";
/// The archive.
pub const URL: &str = "https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/win64/chrome-win64.zip";
pub const FILE: &str = "chrome-win64.zip";
pub const SIZE: u64 = 210_600_182;
pub const SHA256: &str = "ae0deef27d449e1d7529df7705d609bab102de48464da13567ed3dd2ee1a0586";
/// The directory inside the archive.
pub const TOP: &str = "chrome-win64";

/// `<main checkout>/target/chrome-win/<version>/`, beside the `ntdll` cache.
pub fn cache_dir() -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(&crate::root());
    ntdll.parent().map_or_else(|| crate::root().join("target"), Path::to_path_buf).join("chrome-win").join(VERSION)
}

/// Where the unpacked package is (`chrome.exe` is directly in it).
pub fn app_dir() -> PathBuf {
    cache_dir().join(TOP)
}

fn matches(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == SIZE) && crate::sha256::file(path).as_deref() == Some(SHA256)
}

/// Whether the package is fetched and unpacked.
pub fn fetched() -> bool {
    cache_dir().join(".unpacked").is_file() && app_dir().join("chrome.exe").is_file()
}

/// `cargo xtask chrome-win fetch`: the archive (checked) and its contents.
pub fn fetch() -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if fetched() {
        return Ok(app_dir());
    }
    let zip = dir.join(FILE);
    if !matches(&zip) {
        eprintln!("==> downloading {URL}");
        let partial = dir.join(format!("{FILE}.part"));
        let got = Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(URL)
            .status();
        if !got.is_ok_and(|s| s.success()) {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("downloading {URL} failed"));
        }
        std::fs::rename(&partial, &zip).map_err(|e| e.to_string())?;
    }
    if !matches(&zip) {
        let got = crate::sha256::file(&zip).unwrap_or_default();
        let _ = std::fs::remove_file(&zip);
        return Err(format!("{FILE} is not the pinned file (SHA-256 {SHA256}, got {got}); removed"));
    }
    let bytes = std::fs::read(&zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let out = app_dir();
    let _ = std::fs::remove_dir_all(&out);
    eprintln!("==> unpacking {} ({} MiB)", zip.display(), bytes.len() >> 20);
    let started = std::time::Instant::now();
    let mut count = 0;
    crate::unzip::for_each(&bytes, |name, data| {
        let rel = name.strip_prefix(&format!("{TOP}/")).ok_or_else(|| format!("{name}: outside {TOP}/"))?;
        if rel.split('/').any(|c| c.is_empty() || c == ".." || c.contains(':') || c.contains('\\')) {
            return Err(format!("{name}: an unsafe name"));
        }
        let path = out.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        count += 1;
        Ok(())
    })?;
    std::fs::write(dir.join(".unpacked"), format!("{count} files\n")).map_err(|e| e.to_string())?;
    eprintln!("==> {count} files in {} ({:.0?})", out.display(), started.elapsed());
    Ok(out)
}

/// Every file under `dir`, as `(relative path with '/', absolute path)`,
/// sorted.
pub fn walk(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn go(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result {
        let mut entries: Vec<_> =
            std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            if path.is_dir() {
                go(base, &path, out)?;
            } else {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out)?;
    Ok(out)
}

fn is_pe_name(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.ends_with(".exe") || l.ends_with(".dll")
}

// ---- the import roll-up ---------------------------------------------------

/// One PE file of the package and its imports.
pub struct Module {
    pub rel: String,
    pub imports: Vec<Import>,
}

/// Who imports a missing function: the files, whether every import is a
/// delay-load one, whether the coverage map knows the name.
pub type Importers = (BTreeSet<String>, bool, bool);

/// What of the package's imports Poly lacks, rolled up over all its files.
#[derive(Default, Debug)]
pub struct Rollup {
    /// DLL (lower case) -> the package files that import it.
    pub missing_dlls: BTreeMap<String, BTreeSet<String>>,
    /// DLL -> function -> (importing files, delay-load only, the coverage map knows it).
    pub missing_functions: BTreeMap<String, BTreeMap<String, Importers>>,
    pub files: Vec<(String, usize, usize, usize, usize)>,
}

/// Classify every import of `modules`. The package's own DLLs (those in
/// `own`, by lower-case file name, with their exports) count as present;
/// everything else is looked up in `ours`.
pub fn rollup(modules: &[Module], ours: &Ours, cov: &crate::winimports::Coverage, own: &BTreeSet<String>) -> Rollup {
    let mut r = Rollup::default();
    for m in modules {
        let report = Report::new(&m.rel, m.imports.clone(), ours, cov);
        let (all, present, stub, missing, dll) = report.counts();
        r.files.push((m.rel.clone(), all, present + stub, missing, dll));
        for row in &report.rows {
            let dll = row.import.dll.to_ascii_lowercase();
            let base = dll.trim_end_matches(".dll").to_string();
            if own.contains(&format!("{base}.dll")) && !ours.dlls.contains_key(&format!("{base}.dll")) {
                continue;
            }
            match &row.verdict {
                Verdict::MissingDll { host } => {
                    r.missing_dlls.entry(host.clone()).or_default().insert(m.rel.clone());
                }
                Verdict::MissingFunction { host, name, known } => {
                    let e = r.missing_functions.entry(host.clone()).or_default().entry(name.to_string()).or_insert((
                        BTreeSet::new(),
                        true,
                        *known,
                    ));
                    e.0.insert(m.rel.clone());
                    e.1 &= row.import.delay;
                }
                _ => {}
            }
        }
    }
    r
}

impl Rollup {
    pub fn markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "### Files\n\n| File | Imports | Present | Missing functions | Imports of missing DLLs |\n|---|---:|---:|---:|---:|"
        );
        for (f, all, present, missing, dll) in &self.files {
            let _ = writeln!(s, "| `{f}` | {all} | {present} | {missing} | {dll} |");
        }
        let _ = writeln!(s, "\n### Missing DLLs ({})\n", self.missing_dlls.len());
        if self.missing_dlls.is_empty() {
            let _ = writeln!(s, "None.");
        } else {
            let _ = writeln!(s, "| DLL | Imported by |\n|---|---|");
            for (d, by) in &self.missing_dlls {
                let list: Vec<String> = by.iter().map(|f| format!("`{f}`")).collect();
                let _ = writeln!(s, "| `{d}` | {} |", list.join(", "));
            }
        }
        let total: usize = self.missing_functions.values().map(BTreeMap::len).sum();
        let _ = writeln!(s, "\n### Missing functions ({total}, in {} of our DLLs)\n", self.missing_functions.len());
        for (host, fns) in &self.missing_functions {
            let _ = writeln!(s, "#### `{host}` ({})\n", fns.len());
            let _ = writeln!(s, "| Function | Imported by | Load | In the coverage map |\n|---|---|---|---|");
            for (name, (by, delay_only, known)) in fns {
                let list: Vec<String> = by.iter().map(|f| format!("`{f}`")).collect();
                let _ = writeln!(
                    s,
                    "| `{name}` | {} | {} | {} |",
                    list.join(", "),
                    if *delay_only { "delay-load" } else { "static" },
                    if *known { "yes" } else { "no" }
                );
            }
            let _ = writeln!(s);
        }
        s
    }
}

/// Read every PE file of the unpacked package.
pub fn modules() -> Result<Vec<Module>> {
    let dir = app_dir();
    let mut out = Vec::new();
    for (rel, path) in walk(&dir)? {
        if !is_pe_name(&rel) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        match crate::winimports::imports_of(&bytes) {
            Ok(imports) => out.push(Module { rel, imports }),
            Err(e) => eprintln!("==> {rel}: {e} (skipped)"),
        }
    }
    Ok(out)
}

/// The exports of the package's own DLLs (so imports between them bind).
fn own_dlls() -> Result<BTreeMap<String, BTreeMap<String, crate::winimports::Target>>> {
    let mut out = BTreeMap::new();
    for (rel, path) in walk(&app_dir())? {
        if rel.contains('/') || !rel.to_ascii_lowercase().ends_with(".dll") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Ok(exports) = crate::winimports::exports_of(&bytes) {
            out.insert(rel.to_ascii_lowercase(), exports);
        }
    }
    Ok(out)
}

fn imports_command(args: &[String]) -> Result {
    let mut markdown = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--markdown" => markdown = Some(it.next().ok_or("--markdown needs a file")?.clone()),
            other => return Err(format!("chrome-win imports: unknown argument {other}")),
        }
    }
    fetch()?;
    let mods = modules()?;
    let mut ours = crate::winimports::load_ours()?;
    let own = own_dlls()?;
    let own_names: BTreeSet<String> = own.keys().cloned().collect();
    for (name, exports) in own {
        ours.dlls.entry(name).or_insert(exports);
    }
    let all: Vec<Import> = mods.iter().flat_map(|m| m.imports.clone()).collect();
    let cov = crate::winimports::load_coverage(&crate::root(), &ours, &all)?;
    let r = rollup(&mods, &ours, &cov, &own_names);
    let text = r.markdown();
    println!("{text}");
    if let Some(out) = markdown {
        let doc = format!(
            "# Import gaps: Chrome for Testing {VERSION} (win64)\n\n\
             Generated by `cargo xtask chrome-win imports --markdown {out}` (ADR 0155); do not edit by hand.\n\
             Names only: the PE import tables of every `.exe` and `.dll` in the package, followed the way our\n\
             loader binds them (API sets, forwarders, the package's own DLLs).\n\n{text}"
        );
        std::fs::write(&out, doc).map_err(|e| format!("{out}: {e}"))?;
        eprintln!("==> wrote {out}");
    }
    Ok(())
}

// ---- the disk ------------------------------------------------------------

/// The disk image: Chrome as an installed program (ADR 0073: `C:` is
/// `poly/nt/c` on the disk that keeps the registry hives).
pub const DISK_FILE: &str = "poly-chrome-win-disk.img";
/// Where Chrome is on the disk, and as the DOS path programs see.
pub const APP_ON_DISK: &str = "poly/nt/c/Program Files/Chrome";
pub const APP_DOS: &str = "C:\\Program Files\\Chrome";
/// Changes when what the disk carries of Poly's fonts changes (the disk is made again).
const FONTS_STAMP: &str = "fonts-0181-dejavu";
/// The test directory on the disk (`C:\poly-test`): the launcher, the
/// stages, the pages.
pub const TEST_ON_DISK: &str = "poly/nt/c/poly-test";
/// The launcher (`tests/windows/chrome_launch.rs`).
pub const LAUNCHER: &str = "chrome_launch";
/// `C:\Windows\System32` on the disk: `polystub.dll` goes there.
pub const SYSTEM32_ON_DISK: &str = "poly/nt/c/Windows/System32";
/// The spec's first page (stages 1 and 2), `specs/windows/apps/chrome/pages/s1.html`:
/// a script sets a paragraph's text and a box's size from its layout.
pub const S1_PAGE_PATH: &str = "specs/windows/apps/chrome/pages/s1.html";
/// What the dumped DOM holds when the script ran.
pub const DOM_EXPECT: &str = "<p id=\"p\">script-ran 42</p>";
/// The spec's network page (stage 4), `specs/windows/apps/chrome/pages/s4.html`: two frames, one
/// `http://example.com/`, one `https://www.iana.org/`; the paragraph says which loaded.
pub const S4_PAGE_PATH: &str = "specs/windows/apps/chrome/pages/s4.html";
/// What the dumped DOM holds when both frames loaded.
pub const S4_EXPECT: &str = "<p id=\"r\">http=1 https=1</p>";
/// The recorded results of the stages, with the SHA-256 of the page and of
/// each stage's standard output.
pub const SPEC_RESULTS_PATH: &str = "specs/windows/apps/chrome/windows.json";

/// The page's bytes with LF line ends whatever the checkout did to them
/// (the recorded DOM depends on them).
pub fn s1_page() -> Result<Vec<u8>> {
    lf_page(S1_PAGE_PATH)
}

/// The network page's bytes (stage 4), with LF line ends.
pub fn s4_page() -> Result<Vec<u8>> {
    lf_page(S4_PAGE_PATH)
}

fn lf_page(relative: &str) -> Result<Vec<u8>> {
    let path = crate::root().join(relative);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(b);
    }
    Ok(out)
}

/// A stage's recorded standard output (`windows.json`): its length and
/// SHA-256 as lower-case hex. `stage` is the key under `stages`
/// (`"s1"`, `"s2"`).
pub fn spec_stdout(stage: &str) -> Result<(u64, String)> {
    let path = crate::root().join(SPEC_RESULTS_PATH);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let run = &json["stages"][stage]["runs"][0];
    let bytes =
        run["stdout_bytes"].as_u64().ok_or_else(|| format!("{}: no stdout_bytes for {stage}", path.display()))?;
    let sha =
        run["stdout_sha256"].as_str().ok_or_else(|| format!("{}: no stdout_sha256 for {stage}", path.display()))?;
    Ok((bytes, sha.to_string()))
}

// ---- the stage ladder ------------------------------------------------------

/// One rung of the ladder.
pub struct Stage {
    pub number: u32,
    pub name: &'static str,
    /// The program: `None` is Chrome, `Some(name)` a test program beside the launcher
    /// (`tests/windows/<name>.rs`, e.g. the probe of stage 9).
    pub program: Option<&'static str>,
    /// Chrome's command line after the program.
    pub args: &'static [&'static str],
    /// A line the stage's output must contain to pass.
    pub expect: &'static str,
    /// Seconds the launcher waits for Chrome.
    pub seconds: u32,
}

pub const STAGES: [Stage; 5] = [
    Stage {
        number: 1,
        name: "headless dump-dom, no sandbox, no GPU",
        program: None,
        args: &[
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--user-data-dir=C:\\poly-test\\profile",
            "--dump-dom",
            "file:///C:/poly-test/s1.html",
        ],
        expect: DOM_EXPECT,
        seconds: 300,
    },
    Stage {
        number: 2,
        name: "headless dump-dom, sandbox on, no GPU",
        program: None,
        args: &[
            "--headless",
            "--disable-gpu",
            "--user-data-dir=C:\\poly-test\\profile",
            "--dump-dom",
            "file:///C:/poly-test/s1.html",
        ],
        expect: DOM_EXPECT,
        seconds: 300,
    },
    Stage {
        number: 9,
        name: "probe: what Chrome's start-up asks of version.dll, on Chrome's own files",
        program: Some(PROBE),
        args: &[],
        expect: "PROBE file C:\\Program Files\\Chrome\\chrome_elf.dll",
        seconds: 60,
    },
    // S3 (the spec's ladder: a visible window with `--disable-gpu`): Chrome stays open, so the launcher
    // stops it after `seconds` and the stage reports what Chrome printed. The page is stage 1's; the
    // window itself is what `cargo xtask win-app run --mode desktop` shows on the desktop.
    Stage {
        number: 3,
        name: "a visible window, no sandbox, no GPU (ends after the launcher's wait)",
        program: None,
        args: &[
            "--disable-gpu",
            "--no-sandbox",
            "--no-first-run",
            "--user-data-dir=C:\\poly-test\\profile",
            "--window-position=40,40",
            "--window-size=900,500",
            "file:///C:/poly-test/s1.html",
        ],
        expect: "LAUNCH-EXIT",
        seconds: 120,
    },
    // The network stage, headless (the spec's `s4-headless`): the page loads an http and an https
    // frame; the guest has QEMU's user networking.
    Stage {
        number: 4,
        name: "headless dump-dom of the network page (http and https frames), no sandbox, no GPU",
        program: None,
        args: &[
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--user-data-dir=C:\\poly-test\\profile",
            "--dump-dom",
            "file:///C:/poly-test/s4.html",
        ],
        expect: S4_EXPECT,
        seconds: 300,
    },
];

/// The probe of stage 9 (`tests/windows/chrome_probe.rs`).
pub const PROBE: &str = "chrome_probe";

/// What a debugging run changes in the stage files (`chrome-win run --chrome-arg A --launch-seconds N`):
/// more arguments for Chrome (its own logging, `--enable-logging=stderr --v=1`) and a shorter wait,
/// so the launcher prints Chrome's standard error before QEMU is stopped. Empty for every other run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Debugging {
    pub chrome_args: Vec<String>,
    pub seconds: Option<u32>,
    /// The call tracer (`--kit`) and the kernel's NT call trail log only processes whose
    /// command line contains this (`--trace-only`).
    pub trace_only: Option<String>,
    /// The call tracer keeps the last N calls and writes them at a breakpoint (`--trace-ring`).
    pub trace_ring: Option<u32>,
    /// The kernel keeps every process's last N NT calls for a breakpoint (`--trail`).
    pub trail: Option<u32>,
    /// NT services the trail leaves out, comma separated (`--trail-skip`).
    pub trail_skip: Option<String>,
    /// The call tracer writes each function's first call with each outcome only
    /// (`--trace-first`: `POLYTRACE_FIRST`).
    pub trace_first: bool,
    /// The call tracer also writes its ring this many seconds after a process started
    /// (`--ring-at`): what a process that went quiet did last.
    pub ring_at: Option<u32>,
    /// The kernel logs where the frames are every N seconds (`--mem[=N]`: `poly.nt.mem`, ADR 0188).
    pub mem: Option<u32>,
}

static DEBUGGING: std::sync::Mutex<Option<Debugging>> = std::sync::Mutex::new(None);

/// `stage<N>.cmd` for the launcher: the program, its command line, its
/// directory, the seconds to wait (see `tests/windows/chrome_launch.rs`).
pub fn stage_cmd(stage: &Stage) -> String {
    let debugging = DEBUGGING.lock().map(|d| d.clone().unwrap_or_default()).unwrap_or_default();
    stage_cmd_with(stage, &debugging)
}

/// [`stage_cmd`] with a run's [`Debugging`] changes (Chrome's own arguments only: a probe gets none).
pub fn stage_cmd_with(stage: &Stage, debugging: &Debugging) -> String {
    let exe = match stage.program {
        Some(p) => format!("C:\\poly-test\\{p}.exe"),
        None => format!("{APP_DOS}\\chrome.exe"),
    };
    let mut cmdline = format!("\"{exe}\"");
    let extra: &[String] = if stage.program.is_none() { &debugging.chrome_args } else { &[] };
    // Chrome's switches go before the page (its last argument).
    let (page, switches) = match stage.args.split_last() {
        Some((last, rest)) if stage.program.is_none() => (Some(*last), rest),
        _ => (None, stage.args),
    };
    for a in switches.iter().copied().chain(extra.iter().map(String::as_str)).chain(page) {
        cmdline.push(' ');
        cmdline.push_str(a);
    }
    format!("{exe}\n{cmdline}\n{APP_DOS}\n{}\n", debugging.seconds.unwrap_or(stage.seconds))
}

/// The kit's trace file, written by `polytrace.dll` in Chrome's directory.
pub const TRACE_FILE: &str = "polytrace.jsonl";

/// [`stage_cmd`] for a run with the Poly-side call tracer: the logger writes every record when
/// its call returns (`POLYTRACE_SYNC`, so a process that dies leaves its trace) and the launcher
/// sends the trace file to the kernel log afterwards.
///
/// A debugging run can trace only some processes (`--trace-only TEXT`: `POLYTRACE_ONLY`, a
/// process whose command line contains the text) and keep only their last calls, written when
/// they meet a breakpoint (`--trace-ring N`: `POLYTRACE_RING`, ADR 0155 step 11).
pub fn stage_cmd_traced(stage: &Stage) -> String {
    let debugging = DEBUGGING.lock().map(|d| d.clone().unwrap_or_default()).unwrap_or_default();
    format!("{}{}\n{TRACE_FILE}\n", stage_cmd(stage), trace_env(&debugging))
}

/// The launcher's environment line for the call tracer (see [`stage_cmd_traced`]).
pub fn trace_env(debugging: &Debugging) -> String {
    let mut env = match debugging.trace_ring {
        _ if debugging.trace_first => "POLYTRACE_FIRST=1".to_string(),
        Some(n) => format!("POLYTRACE_RING={n}"),
        None => "POLYTRACE_SYNC=1".to_string(),
    };
    if let Some(at) = debugging.ring_at {
        env.push_str(&format!(" POLYTRACE_RING_AT={at}"));
    }
    if let Some(only) = &debugging.trace_only {
        env.push_str(" POLYTRACE_ONLY=");
        env.push_str(only);
    }
    env
}

/// The tree of the disk: Chrome, the launcher, the stages, the test
/// pages, the hives' directory and, when given, `polystub.dll`.
pub fn tree(launcher: &[u8], probe: &[u8], polystub: &[u8], kit: Option<&Path>) -> Result<crate::rootdisk::RootTree> {
    let app = fetch()?;
    let mut tree = crate::rootdisk::RootTree::default();
    tree.dir("poly", 0o755);
    tree.dir("poly/nt", 0o755);
    tree.dir("poly/nt/registry", 0o755);
    tree.dir("poly/nt/c", 0o755);
    // The tracing kit (`cargo xtask chrome-win trace`): its copies of the traced programs replace
    // Chrome's, its proxies and the logger are added beside them.
    let kit_files: Vec<(String, PathBuf)> = match kit {
        Some(dir) => walk(dir)?.into_iter().filter(|(rel, _)| !rel.ends_with(".json")).collect(),
        None => Vec::new(),
    };
    for (rel, path) in walk(&app)? {
        if kit_files.iter().any(|(k, _)| k.eq_ignore_ascii_case(&rel)) {
            continue;
        }
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let low = rel.to_ascii_lowercase();
        let mode = if low.ends_with(".exe") || low.ends_with(".dll") { 0o755 } else { 0o644 };
        tree.host_file(&format!("{APP_ON_DISK}/{rel}"), mode, &path, crate::hostfs::mtime(&meta));
    }
    for (rel, path) in &kit_files {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        tree.host_file(&format!("{APP_ON_DISK}/{rel}"), 0o755, path, crate::hostfs::mtime(&meta));
    }
    tree.file(&format!("{TEST_ON_DISK}/s1.html"), 0o644, &s1_page()?);
    tree.file(&format!("{TEST_ON_DISK}/s4.html"), 0o644, &s4_page()?);
    for stage in &STAGES {
        let cmd = if kit.is_some() { stage_cmd_traced(stage) } else { stage_cmd(stage) };
        tree.file(&format!("{TEST_ON_DISK}/stage{}.cmd", stage.number), 0o644, cmd.as_bytes());
    }
    tree.file(&format!("{TEST_ON_DISK}/{LAUNCHER}.exe"), 0o755, launcher);
    tree.file(&format!("{TEST_ON_DISK}/{PROBE}.exe"), 0o755, probe);
    tree.file(&format!("{SYSTEM32_ON_DISK}/{}", poly_stubs::STUB_DLL), 0o755, polystub);
    // Poly's fonts (ADR 0181): Windows programs get DirectWrite's system collection and GDI's faces from
    // `C:\Windows\Fonts`. Without them Chrome's UI font set-up found no face and started over until its
    // stack was full (stage 1, ADR 0155 addendum 9).
    crate::fonts::fetch_dejavu()?;
    crate::fonts::add_to_root(&mut tree)?;
    Ok(tree)
}

/// `polystub.dll` for the package: a stub for every import of its files
/// that Poly cannot bind (ADR 0155).
pub fn polystub() -> Result<Vec<u8>> {
    let mods = modules()?;
    let mut ours = crate::winimports::load_ours()?;
    let own = own_dlls()?;
    for (name, exports) in own {
        ours.dlls.entry(name).or_insert(exports);
    }
    let all: Vec<Import> = mods.iter().flat_map(|m| m.imports.clone()).collect();
    let cov = crate::winimports::load_coverage(&crate::root(), &ours, &all)?;
    let mut wants = Vec::new();
    for m in &mods {
        for i in &m.imports {
            let verdict = crate::winimports::classify(&i.dll, &i.sym, &ours, &cov);
            if matches!(verdict, Verdict::MissingDll { .. } | Verdict::MissingFunction { .. }) {
                wants.push(poly_stubs::Want { dll: i.dll.clone(), symbol: i.sym.to_string(), kind: None });
            }
        }
    }
    let image = poly_stubs::build(&wants);
    eprintln!("==> polystub.dll: {} stubs, {} KiB", wants.len(), image.len() >> 10);
    Ok(image)
}

/// A fingerprint of what the disk is made of besides Chrome (FNV-1a).
fn fingerprint(parts: &[&[u8]]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for &b in p.iter().chain(&(p.len() as u64).to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// `cargo xtask chrome-win disk [--fresh-disk]`: the disk image, made again
/// when the launcher, `polystub.dll` or a stage changed (or on request).
pub fn disk(fresh: bool) -> Result<PathBuf> {
    disk_with(fresh, None)
}

/// [`disk`] with a tracing kit laid over Chrome's directory (another image file).
pub fn disk_with(fresh: bool, kit: Option<&Path>) -> Result<PathBuf> {
    let name = if kit.is_some() { "poly-chrome-win-trace-disk.img" } else { DISK_FILE };
    let image = crate::root().join("target").join(name);
    let stamp = image.with_extension("stamp");
    let built = crate::winapp::build(&[LAUNCHER, PROBE])?;
    let find = |n: &str| built.iter().find(|(name, _)| name.trim_end_matches(".exe") == n).map(|(_, b)| b.clone());
    let launcher = find(LAUNCHER).ok_or("the launcher was not built")?;
    let probe = find(PROBE).ok_or("the probe was not built")?;
    let stubs = polystub()?;
    let stages: String =
        STAGES.iter().map(|s| if kit.is_some() { stage_cmd_traced(s) } else { stage_cmd(s) }).collect();
    let mut kit_bytes: Vec<u8> = Vec::new();
    if let Some(dir) = kit {
        for (rel, path) in walk(dir)? {
            kit_bytes.extend_from_slice(rel.as_bytes());
            kit_bytes.extend_from_slice(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    let print = fingerprint(&[
        VERSION.as_bytes(),
        &launcher,
        &stubs,
        stages.as_bytes(),
        &s1_page()?,
        &probe,
        &kit_bytes,
        FONTS_STAMP.as_bytes(),
    ]);
    if !fresh && image.exists() && std::fs::read_to_string(&stamp).is_ok_and(|s| s.trim() == print) {
        return Ok(image);
    }
    let tree = tree(&launcher, &probe, &stubs, kit)?;
    let started = std::time::Instant::now();
    crate::rootdisk::write_image_poly_ext2(&tree, &image)?;
    std::fs::write(&stamp, &print).map_err(|e| format!("{}: {e}", stamp.display()))?;
    eprintln!("==> chrome-win disk {} ({} entries, {:.0?})", image.display(), tree.entries.len(), started.elapsed());
    Ok(image)
}

/// What a stage's serial log says.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// The loader's reasons (`ldr: ...` lines of the kernel log), in order.
    pub loader: Vec<String>,
    /// Calls that reached a generated stub (`stub: dll!name`), once each.
    pub stubs: Vec<String>,
    /// The launcher's word on how Chrome ended (`code=0x0`, `timeout ...`).
    pub exit: Option<String>,
    /// Chrome's exit code, when it ended by itself.
    pub code: Option<u32>,
    /// The whole program ended (`POLY-EXIT`).
    pub done: bool,
    pub expected: bool,
    /// The launcher's count of the bytes Chrome wrote to its standard
    /// output and their SHA-256 (`LAUNCH-DIGEST` / `LAUNCH-SHA256` lines).
    pub stdout_bytes: Option<u64>,
    pub stdout_sha256: Option<String>,
    /// Lines of the kernel log that name a fault or an unsupported call.
    pub faults: Vec<String>,
}

impl Summary {
    pub fn parse(log: &str, expect: &str) -> Summary {
        let mut s = Summary::default();
        for line in log.lines() {
            let line = crate::console::strip_ansi(line);
            let line = line.trim_end_matches('\r');
            if let Some(at) = line.find("ldr: ") {
                s.loader.push(line[at..].to_string());
            }
            if let Some(at) = line.find("stub: ") {
                let call = line[at + 6..].trim().to_string();
                if !s.stubs.contains(&call) {
                    s.stubs.push(call);
                }
            }
            if let Some(at) = line.find("LAUNCH-EXIT ") {
                let rest = line[at + 12..].trim().to_string();
                s.code = rest.strip_prefix("code=0x").and_then(|h| u32::from_str_radix(h.trim(), 16).ok());
                s.exit = Some(rest);
            }
            if let Some(at) = line.find("LAUNCH-DIGEST stdout bytes=") {
                s.stdout_bytes = line[at + 27..].trim().parse().ok();
            }
            if let Some(at) = line.find("LAUNCH-SHA256 stdout ") {
                s.stdout_sha256 = Some(line[at + 21..].trim().to_string());
            }
            if line.contains("POLY-EXIT:") {
                s.done = true;
            }
            let lower = line.to_ascii_lowercase();
            if lower.contains("unimplemented")
                || lower.contains("not implemented")
                || lower.contains("unhandled exception")
                || lower.contains("page fault")
                || lower.contains("panic")
            {
                s.faults.push(line.to_string());
            }
            if !expect.is_empty() && line.contains(expect) {
                s.expected = true;
            }
        }
        // The launcher's markers again, from its console output alone: the kernel's own log
        // lines can land in the middle of a marker, and the console wraps long lines with cursor
        // moves (a SHA-256 line is 85 columns), so the lines above may have missed them.
        let stream = console_stream(log);
        let after = |marker: &str| stream.find(marker).map(|at| &stream[at + marker.len()..]);
        if let Some(rest) = after("LAUNCH-EXIT code=0x") {
            let hex: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                s.code = Some(code);
                s.exit = Some(format!("code=0x{hex}"));
            }
        }
        if s.stdout_bytes.is_none()
            && let Some(rest) = after("LAUNCH-DIGEST stdout bytes=")
        {
            s.stdout_bytes = rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok();
        }
        if let Some(rest) = after("LAUNCH-SHA256 stdout ") {
            let hex: String = rest.chars().take_while(char::is_ascii_hexdigit).take(64).collect();
            if hex.len() == 64 {
                s.stdout_sha256 = Some(hex);
            }
        }
        if !expect.is_empty() && stream.contains(expect) {
            s.expected = true;
        }
        s
    }
}

/// The launcher's console output in a serial log as one stream: the kernel's own log lines
/// (`nt: pid ...`, `nt-waits: ...`, written whole and ending in a line break, possibly in the
/// middle of a console line) cut out, then the escape sequences (the console's cursor moves,
/// with which it also wraps long lines) and the line breaks removed.
pub fn console_stream(log: &str) -> String {
    const KERNEL: [&str; 6] = ["nt: pid ", "nt-waits: ", "nt-trace: ", "nt-trail: ", "nt-exc: ", "ptrace"];
    let mut kept = String::with_capacity(log.len());
    let mut rest = log;
    while let Some(at) = KERNEL.iter().filter_map(|k| rest.find(k)).min() {
        kept.push_str(&rest[..at]);
        rest = rest[at..].find('\n').map_or("", |nl| &rest[at + nl + 1..]);
    }
    kept.push_str(rest);
    crate::console::strip_ansi(&kept).chars().filter(|&c| c != '\n').collect()
}

/// Guest memory: Chrome's browser, GPU, network and renderer processes. Stage 1 starts about
/// ten Chrome processes; they share `chrome.dll`'s clean pages since DLLs are image section views
/// (ADR 0188): about 360 MiB of user pages at the most, where every process had its own 300 MB
/// copy before and the 4 GiB guest ran out of frames (ADR 0155 steps 12 and 13).
pub const MEMORY: &str = "4G";
/// Virtual CPUs of a stage.
pub const CPUS: u32 = 4;

/// `cargo xtask chrome-win run [--stage N] [--trace] [--exceptions[=N]] [--waits[=SECS]] [--release]
/// [--fresh-disk] [--timeout SECS] [--chrome-arg ARG]... [--launch-seconds N] [--trace-only TEXT]
/// [--trace-ring N] [--ring-at SECS] [--trace-first] [--trail N] [--trail-skip NAMES] [--mem[=SECS]] [--memory=SIZE]
/// [--cpus=N]`: boot a
/// stage headless (the kernel mounts the disk and runs the launcher as its
/// first program), collect the serial log, print what it says. Works on
/// every host (no QMP). `--exceptions` asks the kernel to log every exception
/// delivered to user mode with its module, offset and N quadwords of the stack
/// (`poly.nt.exceptions`, ADR 0155 step 9): Chrome leaves the reason it stops on its stack.
fn run(args: &[String]) -> Result {
    let (mut stage, mut trace, mut release, mut fresh, mut timeout) = (1u32, false, false, false, 900u64);
    let mut kit_names: Vec<String> = Vec::new();
    let mut exceptions: Option<String> = None;
    let mut waits: Option<String> = None;
    let mut debugging = Debugging::default();
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--stage" => {
                stage = it.next().ok_or("--stage needs a number")?.parse().map_err(|e| format!("--stage: {e}"))?
            }
            "--trace" => trace = true,
            "--waits" => waits = Some(String::new()),
            w if w.starts_with("--waits=") => waits = Some(w["--waits".len()..].to_string()),
            "--exceptions" => exceptions = Some(String::new()),
            e if e.starts_with("--exceptions=") => exceptions = Some(e["--exceptions".len()..].to_string()),
            "--kit" => {
                kit_names = it
                    .next()
                    .ok_or("--kit needs file names (chrome_elf.dll,chrome.exe)")?
                    .split(',')
                    .map(str::to_string)
                    .collect()
            }
            "--release" => release = true,
            "--fresh-disk" => fresh = true,
            "--chrome-arg" => debugging.chrome_args.push(it.next().ok_or("--chrome-arg needs an argument")?.clone()),
            "--trace-only" => debugging.trace_only = Some(it.next().ok_or("--trace-only needs a text")?.clone()),
            "--trace-first" => debugging.trace_first = true,
            "--mem" => debugging.mem = Some(5),
            m if m.starts_with("--mem=") => {
                debugging.mem = Some(m["--mem=".len()..].parse().map_err(|e| format!("--mem: {e}"))?)
            }
            "--trail-skip" => debugging.trail_skip = Some(it.next().ok_or("--trail-skip needs names")?.clone()),
            "--ring-at" => {
                debugging.ring_at =
                    Some(it.next().ok_or("--ring-at needs seconds")?.parse().map_err(|e| format!("--ring-at: {e}"))?)
            }
            "--trail" => {
                debugging.trail =
                    Some(it.next().ok_or("--trail needs a number")?.parse().map_err(|e| format!("--trail: {e}"))?)
            }
            "--trace-ring" => {
                debugging.trace_ring = Some(
                    it.next()
                        .ok_or("--trace-ring needs a number")?
                        .parse()
                        .map_err(|e| format!("--trace-ring: {e}"))?,
                )
            }
            "--launch-seconds" => {
                debugging.seconds = Some(
                    it.next()
                        .ok_or("--launch-seconds needs seconds")?
                        .parse()
                        .map_err(|e| format!("--launch-seconds: {e}"))?,
                )
            }
            "--timeout" => {
                timeout = it.next().ok_or("--timeout needs seconds")?.parse().map_err(|e| format!("--timeout: {e}"))?
            }
            other => rest.push(other.to_string()),
        }
    }
    let st = STAGES
        .iter()
        .find(|s| s.number == stage)
        .ok_or_else(|| format!("no stage {stage} yet (stages: 1..={})", STAGES.len()))?;
    eprintln!("==> stage {}: {}", st.number, st.name);
    if debugging != Debugging::default() {
        eprintln!(
            "==> debugging run: Chrome also gets {:?}, the launcher waits {:?} s",
            debugging.chrome_args, debugging.seconds
        );
    }
    *DEBUGGING.lock().map_err(|e| e.to_string())? = Some(debugging);
    fetch()?;
    let kernel = crate::build_kernel(release)?;
    let kit = if kit_names.is_empty() { None } else { Some(build_kit(&kit_names)?) };
    let outcome = boot_stage(&kernel, st, &Options { trace, exceptions, waits, fresh, timeout, rest, kit })?;
    outcome.report(st);
    if outcome.passed {
        println!("STAGE {} PASSED", st.number);
        Ok(())
    } else {
        Err(format!("stage {} did not pass (see {})", st.number, outcome.log.display()))
    }
}

/// The call-tracing kit (`cargo xtask win trace-kit`, ADR 0090) for the named files of the
/// package: copies whose imports go through proxies that log every call, in
/// `target/chrome-kit`. Only the files named are traced (a program that makes millions of calls
/// per second is too much to trace whole).
pub fn build_kit(names: &[String]) -> Result<PathBuf> {
    let app = fetch()?;
    let exes: Vec<PathBuf> = names.iter().map(|n| app.join(n)).collect();
    if let Some(missing) = exes.iter().find(|p| !p.is_file()) {
        return Err(format!("{}: not in the package", missing.display()));
    }
    let out = crate::root().join("target/chrome-kit");
    crate::wintrace::build_kit_multi(&exes, &out, &[], false, true, &[])?;
    Ok(out)
}

/// The kernel arguments a stage's debugging options add (each with its leading space).
fn kernel_arguments(trace: bool, exceptions: Option<&str>, waits: Option<&str>) -> String {
    let mut more = String::new();
    if trace {
        more.push_str(" poly.nt.trace");
    }
    if let Some(value) = exceptions {
        more.push_str(" poly.nt.exceptions");
        more.push_str(value);
    }
    if let Some(value) = waits {
        more.push_str(" poly.nt.waits");
        more.push_str(value);
    }
    more
}

/// The kernel arguments of a debugging run: `poly.nt.only` for `--trace-only` (the NT call
/// trace and trail of the matching processes only) and `poly.nt.trail=N` for `--trail N`.
pub fn debugging_kernel_arguments(d: &Debugging) -> String {
    let mut more = String::new();
    if let Some(only) = &d.trace_only {
        more.push_str(" poly.nt.only=");
        more.push_str(only);
    }
    if let Some(n) = d.trail {
        more.push_str(&format!(" poly.nt.trail={n}"));
    }
    if let Some(skip) = &d.trail_skip {
        more.push_str(" poly.nt.trail.skip=");
        more.push_str(skip);
    }
    if let Some(n) = d.mem {
        more.push_str(&format!(" poly.nt.mem={n}"));
    }
    more
}

/// How a stage is run.
pub struct Options {
    /// Ask the kernel for the NT call trace (`poly.nt.trace`).
    pub trace: bool,
    /// Ask the kernel to log every exception delivered to user mode
    /// (`poly.nt.exceptions`): `Some("")` for the default stack dump, `Some("=N")` for N
    /// quadwords.
    pub exceptions: Option<String>,
    /// Ask the kernel to log the blocked NT waits every N seconds (`poly.nt.waits`, ADR 0181):
    /// `Some("")` for every 30 s, `Some("=N")` for every N.
    pub waits: Option<String>,
    /// Make the disk again.
    pub fresh: bool,
    /// Seconds before QEMU is stopped.
    pub timeout: u64,
    /// Arguments for `resources::Resources` (`--memory=`, `--cpus=`).
    pub rest: Vec<String>,
    /// A tracing kit to lay over Chrome's directory (`cargo xtask chrome-win trace`).
    pub kit: Option<PathBuf>,
}

/// What a boot of a stage gave.
pub struct Outcome {
    pub summary: Summary,
    pub elapsed: std::time::Duration,
    pub timed_out: bool,
    pub log: PathBuf,
    /// Chrome ended with exit code 0, printed what the stage expects and,
    /// where the spec recorded it, exactly the recorded standard output.
    pub passed: bool,
    /// Why the recorded standard output was not matched, when it was not.
    pub digest_note: Option<String>,
    /// The Poly-side call trace the kit's logger wrote (`ptrace:` lines of the kernel log,
    /// joined), when the run had a kit.
    pub trace: Option<PathBuf>,
}

/// The trace files in a kernel log: the browser's (number 0) and each child process's
/// (`<file>.N`, announced by the launcher's `{"child":N}` record), in the order printed.
pub fn ptrace_files(log: &str) -> Vec<(u32, Vec<String>)> {
    let mut files: Vec<(u32, Vec<String>)> = vec![(0, Vec::new())];
    for record in ptrace_records(log) {
        let child =
            record.strip_prefix("{\"child\":").and_then(|r| r.strip_suffix('}')).and_then(|n| n.parse::<u32>().ok());
        match child {
            Some(n) => files.push((n, Vec::new())),
            None => files.last_mut().expect("never empty").1.push(record),
        }
    }
    files
}

/// The records of the `ptrace:` / `ptrace+` lines of a kernel log (the launcher's
/// `PolyDebugPrint` of the trace files): one per line, continuations joined.
pub fn ptrace_records(log: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in log.lines() {
        let line = crate::console::strip_ansi(line);
        let Some(at) = line.find("ptrace") else { continue };
        let rest = &line[at + 6..];
        if let Some(text) = rest.strip_prefix(": ") {
            out.push(text.trim_end_matches('\r').to_string());
        } else if let Some(text) = rest.strip_prefix("+ ")
            && let Some(last) = out.last_mut()
        {
            last.push_str(text.trim_end_matches('\r'));
        }
    }
    out
}

impl Outcome {
    /// Print what happened.
    pub fn report(&self, st: &Stage) {
        println!(
            "stage {} ({}): {:.0?}{}",
            st.number,
            st.name,
            self.elapsed,
            if self.timed_out { ", TIMED OUT" } else { "" }
        );
        println!("serial log: {}", self.log.display());
        for l in &self.summary.loader {
            println!("  {l}");
        }
        for s in &self.summary.stubs {
            println!("  stub hit: {s}");
        }
        for f in &self.summary.faults {
            println!("  fault: {f}");
        }
        println!(
            "  chrome: {:?}, program ended: {}, expected output: {}",
            self.summary.exit, self.summary.done, self.summary.expected
        );
        match (&self.summary.stdout_bytes, &self.summary.stdout_sha256) {
            (Some(n), Some(sha)) => println!("  stdout: {n} bytes, SHA-256 {sha}"),
            _ => println!("  stdout: no digest from the launcher"),
        }
        if let Some(note) = &self.digest_note {
            println!("  stdout digest: {note}");
        }
        if let Some(t) = &self.trace {
            println!("  poly-side call trace: {}", t.display());
        }
    }
}

/// Whether the standard output the launcher digested is the one the spec
/// recorded for the stage (`windows.json`); `None` when it is, else why not.
fn digest_note(st: &Stage, summary: &Summary) -> Option<String> {
    if st.program.is_some() {
        return None;
    }
    // The spec calls the headless network stage `s4-headless` (its window one is `s4-window`).
    let key = if st.number == 4 { "s4-headless".to_string() } else { format!("s{}", st.number) };
    let (want_bytes, want_sha) = match spec_stdout(&key) {
        Ok(v) => v,
        Err(e) => return Some(e),
    };
    match (summary.stdout_bytes, summary.stdout_sha256.as_deref()) {
        (Some(b), Some(sha)) if b == want_bytes && sha == want_sha => None,
        (Some(b), Some(sha)) => {
            Some(format!("{b} bytes, SHA-256 {sha}; the spec records {want_bytes} bytes, {want_sha}"))
        }
        _ => Some(format!("the launcher printed no digest; the spec records {want_bytes} bytes, {want_sha}")),
    }
}

/// Boot a stage with `kernel` and read what its serial log says.
pub fn boot_stage(kernel: &Path, st: &Stage, o: &Options) -> Result<Outcome> {
    let stage = st.number;
    let image = disk_with(o.fresh, o.kit.as_deref())?;
    // A copy for the run: the hives and the profile are written to it.
    let work = crate::root().join("target/chrome-win");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let run_image = work.join(format!("stage{stage}.img"));
    std::fs::copy(&image, &run_image).map_err(|e| format!("{}: {e}", run_image.display()))?;
    let log = work.join(format!("stage{stage}.serial.log"));
    let _ = std::fs::remove_file(&log);
    let res = crate::resources::Resources::from_args(&o.rest)?;
    eprintln!("{}", res.describe(MEMORY, CPUS));
    let mut more = kernel_arguments(o.trace, o.exceptions.as_deref(), o.waits.as_deref());
    let debugging = DEBUGGING.lock().map(|d| d.clone().unwrap_or_default()).unwrap_or_default();
    more.push_str(&debugging_kernel_arguments(&debugging));
    // The kernel mounts the disk and runs the launcher (NT programs are the
    // first program's business, a Linux shell cannot exec one): no
    // initramfs. The launcher's path has no space; Chrome's does not matter.
    let append = format!(
        "poly.mount=virtio0:/mnt/root poly.init=/mnt/root/poly/nt/c/poly-test/{LAUNCHER}.exe poly.init.arg={stage} \
         poly.qemu_exit console=ttyS0 poly.terminal=off poly.net=qemu{more}"
    );
    let mut cmd = crate::qemu_with(
        kernel,
        &res.memory(MEMORY),
        res.cpus_or(CPUS),
        &append,
        crate::Devices::None,
        &format!("file:{}", log.display()),
    );
    cmd.args(crate::rootdisk::drive_args(&run_image));
    cmd.args(crate::net::nic_args(None, false));
    cmd.args(["-display", "none"]).stdin(std::process::Stdio::null());
    let started = std::time::Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    let limit = std::time::Duration::from_secs(o.timeout);
    let mut timed_out = false;
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let text = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let summary = Summary::parse(&text, st.expect);
    let digest_note = digest_note(st, &summary);
    let passed = summary.expected && summary.code == Some(0) && digest_note.is_none();
    let trace = if o.kit.is_some() {
        // The browser's trace, and `<file>.N` for each child process the launcher printed.
        let file = work.join(format!("stage{stage}.ptrace.jsonl"));
        for n in 1..100 {
            let _ = std::fs::remove_file(work.join(format!("stage{stage}.ptrace.jsonl.{n}")));
        }
        for (child, records) in ptrace_files(&text) {
            let path = if child == 0 { file.clone() } else { work.join(format!("stage{stage}.ptrace.jsonl.{child}")) };
            let mut body = records.join("\n");
            body.push('\n');
            std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Some(file)
    } else {
        None
    };
    Ok(Outcome { summary, elapsed: started.elapsed(), timed_out, log, passed, digest_note, trace })
}

/// The boot test group `chrome-win` (ADR 0155): stage 1, Chrome's headless
/// `--dump-dom`, on the NT personality. Like the 7-Zip group it is skipped
/// when the program is not in the cache (`cargo xtask chrome-win fetch`
/// downloads the 210 MB package; CI has none). Stage 1 must pass
/// ([`S1_REQUIRED`]); with it off the group only reports how far Chrome got,
/// unless `POLY_CHROME_REQUIRED=1`.
pub fn boot_test(kernel: &Path, _syms: &std::sync::Arc<crate::symbols::Symbols>) -> Result {
    eprintln!(
        "==> boot test: Chrome for Testing {VERSION}, headless --dump-dom (stage 1) on the NT personality (ADR 0155)"
    );
    if !fetched() {
        eprintln!(
            "==> skipped: Chrome {VERSION} is not in the cache ({}); `cargo xtask chrome-win fetch` downloads it",
            cache_dir().display()
        );
        return Ok(());
    }
    let st = &STAGES[0];
    let outcome = boot_stage(
        kernel,
        st,
        &Options {
            trace: false,
            exceptions: None,
            waits: None,
            fresh: false,
            timeout: 600,
            rest: Vec::new(),
            kit: None,
        },
    )?;
    outcome.report(st);
    let required = S1_REQUIRED || std::env::var("POLY_CHROME_REQUIRED").is_ok_and(|v| v == "1");
    if outcome.passed {
        println!("STAGE {} PASSED", st.number);
    } else if required {
        return Err(format!("stage {} did not pass (see {})", st.number, outcome.log.display()));
    } else {
        eprintln!(
            "==> stage {} did not pass yet (not required: see S1_REQUIRED in tools/xtask/src/chromewin.rs); log {}",
            st.number,
            outcome.log.display()
        );
    }
    stage3_window(kernel)
}

/// Stage 3's page: green, so a pixel of the page area shows that the window is up and painted.
const S3_PAGE: &str =
    "data:text/html,<title>poly-s3</title><style>body{background:green;color:white}</style><h1>HelloPoly</h1>";

/// The steps of the stage 3 check, on the desktop: start Chrome; stop/reload the page twice at 50 s
/// (a slow start still finishes loading, so the component updater's download task runs, which
/// stopped the browser about 35 s after its first page before ADR 0207); at about
/// 125 s the page area, (200, 400) of the 1280x800 screen, must be `green` (CSS: 008000).
const S3_STEPS: &str = "launch,sleep:50,click:143|103,sleep:3,click:143|103,sleep:70,pixel:200|400|008000";

/// What one stage 3 run showed.
#[derive(Debug, PartialEq, Eq)]
enum S3Verdict {
    /// The page is painted and the browser threw no exception.
    Pass,
    /// The page area is still blank white: a slow start whose page never finished loading (the
    /// component updater does not even run then), so the run says nothing either way.
    Inconclusive,
    Fail,
}

/// Judge a run: an exception delivered to user mode, a window that never came, or any colour but
/// the page's or blank white at the pixel is a failure.
fn s3_verdict(stopped: Option<&str>, exceptions: &[&str]) -> S3Verdict {
    match stopped {
        _ if !exceptions.is_empty() => S3Verdict::Fail,
        None => S3Verdict::Pass,
        Some(why) if why.contains("is [ff, ff, ff], wanted") => S3Verdict::Inconclusive,
        Some(_) => S3Verdict::Fail,
    }
}

/// The stage 3 boot check: the browser's window, with the GPU off, shows its page and is still up
/// after the component updater's BITS download was refused (ADR 0207: the apartment of a COM
/// thread is in `TEB.ReservedForOle`). Skipped where there is no Unix host or no cached Chrome.
/// An exception or a missing window fails it at once; a page that never finished loading (blank
/// white, the slow start that sometimes happens under load) repeats the run, up to three times,
/// and is reported as inconclusive after that: such a run cannot show the browser's break.
fn stage3_window(kernel: &Path) -> Result {
    eprintln!("==> boot test: Chrome for Testing {VERSION}, a window with its page painted after 125 s (stage 3)");
    if !cfg!(unix) {
        eprintln!("==> skipped: a desktop run needs a Unix host (QMP over a Unix socket)");
        return Ok(());
    }
    let dir = cache_dir().join(TOP);
    let args = format!(
        "--disable-gpu --no-sandbox --no-first-run --user-data-dir=C:\\poly-prof --window-position=40,40 --window-size=900,500 {S3_PAGE}"
    );
    let log = crate::root().join("target/win-app/s3-boot/chrome.serial.log");
    for attempt in 1..=3 {
        let (text, stopped) =
            crate::winapps::desktop_check(kernel, &dir, "chrome.exe", "chrome", &args, S3_STEPS, &log)?;
        let exceptions: Vec<&str> = text.lines().filter(|l| l.contains("nt-exc:")).take(3).collect();
        match s3_verdict(stopped.as_deref(), &exceptions) {
            S3Verdict::Pass => {
                println!("STAGE 3 PASSED");
                return Ok(());
            }
            S3Verdict::Inconclusive => {
                eprintln!(
                    "==> stage 3, attempt {attempt}: the page was still loading (slow start); log {}",
                    log.display()
                );
            }
            S3Verdict::Fail => {
                return Err(format!(
                    "stage 3 did not pass: {}; exceptions: {exceptions:?} (see {})",
                    stopped.unwrap_or_else(|| "the steps ran".into()),
                    log.display()
                ));
            }
        }
    }
    println!("STAGE 3 INCONCLUSIVE: the page never finished loading in 3 runs (slow start); no exception either");
    Ok(())
}

/// Whether the `chrome-win` boot test group fails when stage 1 does not
/// pass. It passes every time since ADR 0155 step 13 (the failed check at
/// `chrome.dll+0xc62f809` and the timeouts were a looping `InterlockedPushEntrySList`).
pub const S1_REQUIRED: bool = true;

/// `cargo xtask chrome-win <fetch | imports | disk | run>`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch") => fetch().map(|dir| println!("{}", dir.display())),
        Some("imports") => imports_command(&args[1..]),
        Some("disk") => disk(args.iter().any(|a| a == "--fresh-disk")).map(|p| println!("{}", p.display())),
        Some("run") => run(&args[1..]),
        _ => Err("usage: cargo xtask chrome-win <fetch | imports [--markdown FILE] | disk [--fresh-disk] | \
                  run [--stage N] [--trace] [--exceptions[=N]] [--waits[=SECS]] [--kit FILES] [--release] [--timeout SECS]>"
            .into()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stage_3_is_judged_by_exceptions_and_the_pixel() {
        use super::{S3Verdict, s3_verdict};
        let white = "Pixel(200, 400, [0, 128, 0]): pixel 200,400 is [ff, ff, ff], wanted [00, 80, 00]";
        let wallpaper = "Pixel(200, 400, [0, 128, 0]): pixel 200,400 is [30, 30, 30], wanted [00, 80, 00]";
        assert_eq!(s3_verdict(None, &[]), S3Verdict::Pass);
        assert_eq!(s3_verdict(None, &["nt-exc: pid 2 tid 21 code 0x80000003"]), S3Verdict::Fail);
        assert_eq!(s3_verdict(Some(white), &[]), S3Verdict::Inconclusive);
        assert_eq!(s3_verdict(Some(white), &["nt-exc: pid 2"]), S3Verdict::Fail);
        assert_eq!(s3_verdict(Some(wallpaper), &[]), S3Verdict::Fail);
        assert_eq!(s3_verdict(Some("Launch: no \"poly-compositor: launched\" in the log"), &[]), S3Verdict::Fail);
    }

    #[test]
    fn debugging_options_become_kernel_arguments() {
        assert_eq!(kernel_arguments(false, None, None), "");
        assert_eq!(kernel_arguments(true, None, None), " poly.nt.trace");
        assert_eq!(kernel_arguments(false, Some(""), None), " poly.nt.exceptions");
        assert_eq!(kernel_arguments(false, None, Some("=20")), " poly.nt.waits=20");
        assert_eq!(kernel_arguments(true, Some("=1536"), None), " poly.nt.trace poly.nt.exceptions=1536");
    }

    #[test]
    fn the_kernel_log_holds_the_browsers_trace_and_one_per_child() {
        let log = "boot\n\
                   nt: pid 1: ptrace: {\"format\":\"polytrace/1\"}\r\n\
                   nt: pid 1: ptrace: {\"seq\":1,\"fn\":\"A\",\"args\":[\"lo\r\n\
                   nt: pid 1: ptrace+ ng\"]}\r\n\
                   noise\n\
                   nt: pid 1: ptrace: {\"child\":1}\r\n\
                   nt: pid 1: ptrace: {\"seq\":1,\"fn\":\"B\"}\r\n\
                   nt: pid 1: ptrace: {\"child\":12}\r\n";
        let files = ptrace_files(log);
        assert_eq!(files.len(), 3);
        assert_eq!(
            files[0],
            (
                0,
                vec![
                    "{\"format\":\"polytrace/1\"}".to_string(),
                    "{\"seq\":1,\"fn\":\"A\",\"args\":[\"long\"]}".to_string()
                ]
            )
        );
        assert_eq!(files[1], (1, vec!["{\"seq\":1,\"fn\":\"B\"}".to_string()]));
        assert_eq!(files[2], (12, Vec::new()));
        // A log without a kit has one empty file.
        assert_eq!(ptrace_files("boot\n"), vec![(0, Vec::new())]);
    }

    use super::*;
    use crate::winimports::{Coverage, Sym, Target};

    fn ours() -> Ours {
        let mut o = Ours::default();
        let mut k = BTreeMap::new();
        k.insert("CreateFileW".to_string(), Target::Code);
        o.dlls.insert("kernel32.dll".into(), k);
        o
    }

    #[test]
    fn rolls_up_missing_dlls_and_functions_across_files() {
        let imp = |dll: &str, f: &str, delay| Import { dll: dll.into(), sym: Sym::Name(f.into()), delay };
        let mods = vec![
            Module {
                rel: "chrome.exe".into(),
                imports: vec![imp("KERNEL32.dll", "CreateFileW", false), imp("KERNEL32.dll", "Nope", false)],
            },
            Module {
                rel: "chrome.dll".into(),
                imports: vec![
                    imp("KERNEL32.dll", "Nope", true),
                    imp("dxgi.dll", "CreateDXGIFactory", true),
                    imp("chrome_elf.dll", "Anything", false),
                ],
            },
        ];
        let own: BTreeSet<String> = ["chrome_elf.dll".to_string()].into();
        let r = rollup(&mods, &ours(), &Coverage::default(), &own);
        assert_eq!(r.missing_dlls.keys().collect::<Vec<_>>(), ["dxgi.dll"]);
        let nope = &r.missing_functions["kernel32.dll"]["Nope"];
        assert_eq!(nope.0.len(), 2);
        // Static in one file: not delay-load only.
        assert!(!nope.1);
        let md = r.markdown();
        assert!(md.contains("| `dxgi.dll` | `chrome.dll` |"), "{md}");
        assert!(md.contains("#### `kernel32.dll` (1)"), "{md}");
    }

    #[test]
    fn the_stage_file_names_the_program_its_line_and_the_wait() {
        let cmd = stage_cmd(&STAGES[0]);
        let lines: Vec<&str> = cmd.lines().collect();
        assert_eq!(lines[0], "C:\\Program Files\\Chrome\\chrome.exe");
        assert!(lines[1].starts_with("\"C:\\Program Files\\Chrome\\chrome.exe\" --headless --no-sandbox"), "{cmd}");
        assert!(lines[1].ends_with("--dump-dom file:///C:/poly-test/s1.html"), "{cmd}");
        assert_eq!(lines[2], "C:\\Program Files\\Chrome");
        assert_eq!(lines[3], "300");
        // The sandbox stage is the same without the flag.
        assert!(!stage_cmd(&STAGES[1]).contains("--no-sandbox"));
        // A debugging run's switches go before the page; its wait replaces the stage's.
        let d = Debugging { chrome_args: vec!["--v=1".into()], seconds: Some(40), ..Debugging::default() };
        let cmd = stage_cmd_with(&STAGES[0], &d);
        let lines: Vec<&str> = cmd.lines().collect();
        assert!(lines[1].ends_with("--dump-dom --v=1 file:///C:/poly-test/s1.html"), "{cmd}");
        assert_eq!(lines[3], "40");
        // A probe gets no Chrome switches.
        assert!(!stage_cmd_with(&STAGES[2], &d).contains("--v=1"));
        // The tracer's environment: every call written, or a filtered ring.
        assert_eq!(trace_env(&Debugging::default()), "POLYTRACE_SYNC=1");
        let ring = Debugging { trace_only: Some("--type=renderer".into()), trace_ring: Some(500), ..d };
        assert_eq!(trace_env(&ring), "POLYTRACE_RING=500 POLYTRACE_ONLY=--type=renderer");
        let kernel = Debugging { trail: Some(128), ..ring };
        assert_eq!(debugging_kernel_arguments(&kernel), " poly.nt.only=--type=renderer poly.nt.trail=128");
        let mem = Debugging { mem: Some(3), ..Debugging::default() };
        assert_eq!(debugging_kernel_arguments(&mem), " poly.nt.mem=3");
    }

    #[test]
    fn the_summary_reads_a_log() {
        let log = "boot...\nnt: pid 5: ldr: process start failed (0xc0000139): chrome.exe: kernel32.dll!Foo not found (0xc0000139)\n\
                   nt: pid 5: stub: user32!SetFoo\nnt: pid 5: stub: user32!SetFoo\nLAUNCH-EXIT code=0x3\nPOLY-EXIT: pid=2 code=0\n";
        let s = Summary::parse(log, DOM_EXPECT);
        assert_eq!(s.loader.len(), 1);
        assert!(s.loader[0].contains("kernel32.dll!Foo not found"));
        assert_eq!(s.stubs, ["user32!SetFoo"]);
        assert_eq!(s.code, Some(3));
        assert!(s.done && !s.expected);
        let ok = Summary::parse(
            "<html><p id=\"p\">script-ran 42</p>\nLAUNCH-DIGEST stdout bytes=440\nLAUNCH-SHA256 stdout abc123\nLAUNCH-EXIT code=0x0\n",
            DOM_EXPECT,
        );
        assert!(ok.expected && ok.code == Some(0));
        assert_eq!((ok.stdout_bytes, ok.stdout_sha256.as_deref()), (Some(440), Some("abc123")));
        // The kernel's lines inside the console's, and the console's own wrap (ADR 0155 step 12:
        // the first stage 1 run that printed the DOM).
        let mixed = "LAUNCH-BEGIN stdout\x1b[3;1H<p id=\"p\">script-ran 42</p>\x1b[18;1HLAUNCH-END stdout\x1b[19;1H\
                     LAUNCH-DIGEST stdout bytes=440\x1b[20;1HLAUNCH-SHA256 nt: pid 3: stub: kernel32!GetThreadPriority\r\n\
                     stdout 7b79f8b475nt: pid 3: ldr: LdrLoadDll(x) failed (0xc0000135)\r\n\
                     542cce0f029ef2459918c36404fe1986144f2fd1f2bf8aea1\x1b[21;1H3807b\x1b[22;1H\
                     LAUNCH-EXIT code=0nt-waits: p3 ends a thread status 0x0\r\nx0\x1b[23;1H\n";
        let m = Summary::parse(mixed, DOM_EXPECT);
        assert_eq!(m.stdout_bytes, Some(440));
        assert_eq!(
            m.stdout_sha256.as_deref(),
            Some("7b79f8b475542cce0f029ef2459918c36404fe1986144f2fd1f2bf8aea13807b")
        );
        assert_eq!((m.code, m.expected), (Some(0), true));
        assert!(m.loader.iter().any(|l| l.contains("LdrLoadDll(x)")));
        let late = Summary::parse("LAUNCH-EXIT timeout seconds=300\n", DOM_EXPECT);
        assert_eq!(late.exit.as_deref(), Some("timeout seconds=300"));
        assert_eq!(late.code, None);
    }

    #[test]
    fn the_spec_records_stage_one_and_its_page() {
        let (bytes, sha) = spec_stdout("s1").unwrap();
        assert_eq!(bytes, 440);
        assert_eq!(sha, "7b79f8b475542cce0f029ef2459918c36404fe1986144f2fd1f2bf8aea13807b");
        // Stage 2 records the same output; the page is the one the digest is of.
        assert_eq!(spec_stdout("s2").unwrap(), (bytes, sha));
        let text = std::fs::read_to_string(crate::root().join(SPEC_RESULTS_PATH)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        let page = json["pages"]["s1.html"].as_str().unwrap();
        assert_eq!(crate::sha256::hex(&s1_page().unwrap()), page);
        assert!(spec_stdout("s9").is_err());
    }

    #[test]
    fn stage_one_and_two_run_the_specs_page() {
        for st in STAGES.iter().filter(|s| s.program.is_none() && s.number <= 2) {
            assert!(st.args.contains(&"file:///C:/poly-test/s1.html"));
            assert!(st.args.iter().any(|a| a.starts_with("--user-data-dir=")));
            assert!(st.expect.contains("script-ran 42"));
        }
    }

    #[test]
    fn stage_four_runs_the_network_page_and_the_spec_records_its_output() {
        let st = STAGES.iter().find(|s| s.number == 4).unwrap();
        assert!(st.args.contains(&"file:///C:/poly-test/s4.html"));
        assert!(st.args.contains(&"--no-sandbox"));
        assert_eq!(st.expect, S4_EXPECT);
        // The page the spec recorded the digest of, and the digest: 831 bytes.
        let text = std::fs::read_to_string(crate::root().join(SPEC_RESULTS_PATH)).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(crate::sha256::hex(&s4_page().unwrap()), json["pages"]["s4.html"].as_str().unwrap());
        assert_eq!(spec_stdout("s4-headless").unwrap().0, 831);
        let mut s = Summary {
            stdout_bytes: Some(831),
            stdout_sha256: Some(spec_stdout("s4-headless").unwrap().1),
            ..Summary::default()
        };
        assert_eq!(digest_note(st, &s), None);
        s.stdout_bytes = Some(440);
        assert!(digest_note(st, &s).unwrap().contains("the spec records 831 bytes"));
    }

    #[test]
    fn a_digest_other_than_the_recorded_one_is_not_a_pass() {
        let mut s = Summary { stdout_bytes: Some(440), stdout_sha256: Some("00".into()), ..Summary::default() };
        assert!(digest_note(&STAGES[0], &s).unwrap().contains("the spec records 440 bytes"));
        s.stdout_sha256 = Some(spec_stdout("s1").unwrap().1);
        assert_eq!(digest_note(&STAGES[0], &s), None);
        s.stdout_bytes = None;
        assert!(digest_note(&STAGES[0], &s).unwrap().contains("no digest"));
    }

    #[test]
    fn only_pe_names_are_read() {
        assert!(is_pe_name("chrome.exe") && is_pe_name("Dir/X.DLL"));
        assert!(!is_pe_name("chrome.pak") && !is_pe_name("resources.pak"));
    }
}
