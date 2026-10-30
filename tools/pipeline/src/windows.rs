//! The Windows test oracle (ADR 0075): Windows test programs built here,
//! run on a real Windows machine by `tools/pipeline/windows/poly-oracle.ps1`
//! and on Poly's NT personality in QEMU, and the two runs compared.
//!
//! ```text
//! tests/windows/*.rs, specs/windows/<component>/<symbol>.tests/*.rs   test sources (Rust, no_std)
//!   │ win build: cargo -Zbuild-std for x86_64-pc-windows-msvc, rust-lld (lld-link)
//!   │            (and `*.cpp`: clang++ and lld-link, `wincxx`, ADR 0098)
//!   ▼
//! target/pipeline/windows/bin/<test>.exe + manifest.json (poly-winbuild/1)
//!   │ win bundle: one text file (poly-winbundle/1), base64 lines
//!   ├──────────────► Windows: poly-oracle.ps1 ─► results.json (poly-winrun/1)
//!   └ win poly ────► Poly in QEMU ──────────────► poly.json    (poly-winrun/1)
//!                       win diff: per test, exit code + observation file + stdout + stderr
//! ```
//!
//! A test writes its observations into `poly-test.out` beside itself (the
//! support library `tests/windows/support`, crate `poly-wtest`); both
//! runners read that file. Nothing here observes Windows itself: the
//! results come from the owner's machine as a file.

use crate::poly;
use crate::sha256;
use crate::util::{self, Result};
use crate::wincxx;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where `win` commands put their files, relative to the repository root.
pub const WIN_DIR: &str = "target/pipeline/windows";
/// The corpus of Windows test programs.
pub const CORPUS_DIR: &str = "tests/windows";
/// The support library every test links.
pub const SUPPORT_CRATE: &str = "tests/windows/support";
pub const TARGET: &str = "x86_64-pc-windows-msvc";
/// The only DLLs a test program may import from (by name).
pub const ALLOWED_DLLS: &[&str] = &["ntdll.dll", "kernel32.dll"];
/// The observation file a test writes beside itself (`poly_wtest::OUTPUT_FILE`).
pub const OUTPUT_FILE: &str = "poly-test.out";

pub const BUILD_FORMAT: &str = "poly-winbuild/1";
pub const BUNDLE_FORMAT: &str = "poly-winbundle/1";
pub const RUN_FORMAT: &str = "poly-winrun/1";
pub const DIFF_FORMAT: &str = "poly-windiff/1";

// ---------------------------------------------------------------------------
// Sources and the build

/// A test program's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The file stem: the test's name and the executable's.
    pub name: String,
    pub path: PathBuf,
}

impl Source {
    /// A C++ test (`.cpp`, built by [`wincxx`]) rather than a Rust one.
    pub fn is_cpp(&self) -> bool {
        self.path.extension().is_some_and(|x| x == "cpp")
    }
}

/// The extensions of test sources: Rust (ADR 0075) and C++ (ADR 0098).
pub const TEST_EXTENSIONS: &[&str] = &["rs", "cpp"];

/// Whether `path` names a Windows test source.
pub fn is_test_source(path: &Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| TEST_EXTENSIONS.contains(&x))
}

/// A test name is a Rust identifier in lower case (it is a Cargo binary
/// and a file name on both systems).
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The `.rs` and `.cpp` files of the corpus and of every `extra` directory
/// (a spec entry's visible or held-out tests), sorted by name. Names must
/// be unique and valid.
pub fn sources(repo: &Path, extra: &[PathBuf]) -> Result<Vec<Source>> {
    let mut dirs = vec![repo.join(CORPUS_DIR)];
    dirs.extend(extra.iter().map(|d| util::absolute(d)));
    let mut out: Vec<Source> = Vec::new();
    for dir in dirs {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in entries {
            let path = e.map_err(|e| e.to_string())?.path();
            if !path.is_file() || !is_test_source(&path) {
                continue;
            }
            let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if !valid_name(&name) {
                return Err(format!("{}: a test name is lower-case letters, digits and _", path.display()));
            }
            if let Some(other) = out.iter().find(|s| s.name == name) {
                return Err(format!("two tests named {name}: {} and {}", other.path.display(), path.display()));
            }
            out.push(Source { name, path });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// A TOML basic string (the paths Cargo reads).
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The generated package that builds every test as a binary of its own.
pub fn cargo_toml(support: &Path, sources: &[Source]) -> String {
    let mut s = String::from(
        "# Generated by `cargo xtask pipeline win build` (ADR 0075); do not edit.\n\
         [workspace]\n\n\
         [package]\n\
         name = \"poly-wtests\"\n\
         version = \"0.0.0\"\n\
         edition = \"2024\"\n\
         publish = false\n\
         autobins = false\n\
         autoexamples = false\n\
         autotests = false\n\
         autobenches = false\n\n\
         [dependencies]\n",
    );
    s.push_str(&format!("poly-wtest = {{ path = {} }}\n", toml_str(&support.to_string_lossy())));
    for src in sources {
        s.push_str(&format!(
            "\n[[bin]]\nname = {}\npath = {}\ntest = false\nbench = false\n",
            toml_str(&src.name),
            toml_str(&src.path.to_string_lossy())
        ));
    }
    s.push_str(
        "\n[profile.release]\npanic = \"abort\"\nopt-level = \"s\"\ncodegen-units = 1\ndebug = false\nlto = true\n\
         \n[profile.dev]\npanic = \"abort\"\n",
    );
    s
}

/// Compiler flags: `rust-lld` as `lld-link`, no default libraries, our
/// entry point, a console program, a reproducible image (`/Brepro`: no
/// time stamp; no PDB) and no build-machine paths in panic locations.
pub fn rustflags(repo: &Path, sysroot: &Path) -> Vec<String> {
    let mut f: Vec<String> = [
        "-Clinker=rust-lld",
        "-Clinker-flavor=lld-link",
        "-Clink-arg=/NODEFAULTLIB",
        "-Clink-arg=/ENTRY:mainCRTStartup",
        "-Clink-arg=/SUBSYSTEM:CONSOLE",
        "-Clink-arg=/Brepro",
        "-Clink-arg=/DEBUG:NONE",
        "-Cdebuginfo=0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    f.push(format!("--remap-path-prefix={}=/poly", repo.display()));
    f.push(format!("--remap-path-prefix={}=/rust", sysroot.display()));
    f
}

/// One built test program.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Built {
    pub name: String,
    /// The source, relative to the repository if it is inside it.
    pub source: String,
    pub source_sha256: String,
    pub exe: String,
    pub exe_sha256: String,
    pub size: u64,
    /// DLL (lower case) to the names imported from it.
    pub imports: BTreeMap<String, Vec<String>>,
}

/// `manifest.json` beside the executables.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub format: String,
    /// `rustc -V` of the toolchain that built them.
    pub rustc: String,
    /// `git rev-parse HEAD` of the repository, if known.
    #[serde(default)]
    pub revision: Option<String>,
    /// `clang++ --version` of the compiler of the C++ tests, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clang: Option<String>,
    pub tests: Vec<Built>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Manifest> {
        let m: Manifest = util::read_json(&dir.join("manifest.json"))?;
        if m.format != BUILD_FORMAT {
            return Err(format!("{}: not a {BUILD_FORMAT} manifest", dir.display()));
        }
        Ok(m)
    }
}

/// What a test executable must be: an x64 PE32+ console executable that
/// imports by name only, only from `allowed` ([`ALLOWED_DLLS`]; C++ tests
/// also `msvcrt.dll`, [`wincxx::ALLOWED_DLLS`]), with no delay-load
/// imports. Returns its imports.
pub fn check_image_with(bytes: &[u8], allowed: &[&str]) -> Result<BTreeMap<String, Vec<String>>> {
    let pe = poly_pe::Pe::parse(bytes).map_err(|e| format!("not a PE image: {e:?}"))?;
    let info = pe.info();
    if info.machine() != poly_pe::machine::AMD64 || info.bits() != poly_pe::Bits::Pe32Plus {
        return Err("not an x64 PE32+ image".into());
    }
    if info.is_dll() || !info.is_executable() {
        return Err("not an executable".into());
    }
    if info.optional.subsystem != poly_pe::subsystem::WINDOWS_CUI {
        return Err(format!("subsystem {} is not the console (3)", info.optional.subsystem));
    }
    if pe.delay_imports().map_err(|e| format!("delay imports: {e:?}"))?.is_some_and(|mut d| d.next().is_some()) {
        return Err("delay-load imports are not allowed".into());
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let Some(descriptors) = pe.imports().map_err(|e| format!("imports: {e:?}"))? else { return Ok(out) };
    for d in descriptors {
        let d = d.map_err(|e| format!("imports: {e:?}"))?;
        let dll = String::from_utf8_lossy(d.dll_name().map_err(|e| format!("{e:?}"))?).to_ascii_lowercase();
        if !allowed.contains(&dll.as_str()) {
            return Err(format!("imports from {dll}; only {} are allowed", allowed.join(" and ")));
        }
        let names = out.entry(dll.clone()).or_default();
        for e in d.entries().map_err(|e| format!("{e:?}"))? {
            match e.map_err(|e| format!("{dll}: {e:?}"))?.symbol {
                poly_pe::ImportSymbol::Name { name, .. } => names.push(String::from_utf8_lossy(name).into_owned()),
                poly_pe::ImportSymbol::Ordinal(n) => {
                    return Err(format!("imports ordinal {n} from {dll}; import by name only"));
                }
            }
        }
        names.sort();
    }
    Ok(out)
}

fn output_of(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn rel(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo).unwrap_or(path).to_string_lossy().into_owned()
}

/// Build every test of [`sources`] (only those named in `only`, if any)
/// into `out` (`<name>.exe` and `manifest.json`; older executables there
/// are removed). Cargo's own files go to `WIN_DIR/cargo`: the first build
/// compiles `core` for the target (about half a minute), later ones only
/// the tests.
pub fn build(repo: &Path, out: &Path, extra: &[PathBuf], only: &[String]) -> Result<Manifest> {
    let mut srcs = sources(repo, extra)?;
    if !only.is_empty() {
        for name in only {
            if !srcs.iter().any(|s| &s.name == name) {
                return Err(format!("no Windows test {name}"));
            }
        }
        srcs.retain(|s| only.contains(&s.name));
    }
    if srcs.is_empty() {
        return Err("no Windows test programs to build".into());
    }
    let (cpp, rust): (Vec<Source>, Vec<Source>) = srcs.iter().cloned().partition(Source::is_cpp);
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let sysroot = PathBuf::from(output_of(Command::new(&rustc).current_dir(repo).args(["--print", "sysroot"]))?);
    let version = output_of(Command::new(&rustc).current_dir(repo).arg("-V"))?;
    let win = repo.join(WIN_DIR);
    let target_dir = win.join("cargo");
    if !rust.is_empty() {
        build_rust(repo, &win, &target_dir, &sysroot, &rust)?;
    }
    let tools = wincxx::Tools::from_env();
    let cxx_dir = win.join("cxx");
    let (clang, libs) = if cpp.is_empty() {
        (None, Vec::new())
    } else {
        eprintln!("==> build {} C++ Windows test programs", cpp.len());
        (Some(tools.version()?), wincxx::import_libraries(repo, &tools, &cxx_dir)?)
    };
    util::create_dir(out)?;
    for e in std::fs::read_dir(out).map_err(|e| format!("{}: {e}", out.display()))? {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "exe") {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    let mut tests = Vec::new();
    for src in &srcs {
        let exe = format!("{}.exe", src.name);
        let (built, allowed) = if src.is_cpp() {
            (wincxx::build_one(repo, &tools, &libs, &src.name, &src.path, &cxx_dir)?, wincxx::ALLOWED_DLLS)
        } else {
            (target_dir.join(TARGET).join("release").join(&exe), ALLOWED_DLLS)
        };
        let bytes = util::read(&built)?;
        let imports = check_image_with(&bytes, allowed).map_err(|e| format!("{exe}: {e}"))?;
        util::write(&out.join(&exe), &bytes)?;
        tests.push(Built {
            name: src.name.clone(),
            source: rel(repo, &src.path),
            source_sha256: sha256::hex(&util::read(&src.path)?),
            exe,
            exe_sha256: sha256::hex(&bytes),
            size: bytes.len() as u64,
            imports,
        });
    }
    let revision = util::git_text(repo, &["rev-parse", "HEAD"]).ok().map(|s| s.trim().to_string());
    let m = Manifest { format: BUILD_FORMAT.into(), rustc: version, revision, clang, tests };
    util::write_json(&out.join("manifest.json"), &m)?;
    Ok(m)
}

/// The Rust tests: one generated Cargo package with a binary per test.
fn build_rust(repo: &Path, win: &Path, target_dir: &Path, sysroot: &Path, srcs: &[Source]) -> Result<()> {
    let vendor = sysroot.join("lib/rustlib/src/rust/library/vendor");
    if !vendor.is_dir() {
        return Err(format!("{} not found: the rust-src component is needed", vendor.display()));
    }
    let gen_dir = win.join("build");
    util::write(&gen_dir.join("Cargo.toml"), cargo_toml(&repo.join(SUPPORT_CRATE), srcs))?;
    let flags = rustflags(repo, sysroot).join("\u{1f}");
    eprintln!("==> build {} Windows test programs", srcs.len());
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(repo)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .env_remove("RUSTFLAGS")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(gen_dir.join("Cargo.toml"))
        .args(["-Zbuild-std=core,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", TARGET])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={}", toml_str(&vendor.to_string_lossy())))
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building the Windows test programs failed ({status})"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Base64 (RFC 4648, with padding): the bundle and the result files.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn unbase64(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(4) {
        return Err("base64: length is not a multiple of 4".into());
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let value = |c: u8| -> Result<u32> {
        B64.iter()
            .position(|&x| x == c)
            .map(|p| p as u32)
            .ok_or_else(|| format!("base64: bad character {:?}", c as char))
    };
    let bytes = text.as_bytes();
    for (i, q) in bytes.chunks(4).enumerate() {
        let last = i == bytes.len() / 4 - 1;
        let pad = q.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return Err("base64: misplaced padding".into());
        }
        let mut n = 0u32;
        for &c in &q[..4 - pad] {
            n = n << 6 | value(c)?;
        }
        n <<= 6 * pad as u32;
        let decoded = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&decoded[..3 - pad]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The bundle: what travels to the Windows machine.

/// The bundle text of `manifest` and the executables in `dir`:
///
/// ```text
/// poly-winbundle/1
/// manifest <base64 of manifest.json>
/// file <name>.exe <sha256> <base64>
/// end <files>
/// ```
///
/// Line based so a long bundle can be cut into parts at line ends and the
/// parts concatenated again; every file carries its hash.
pub fn bundle(manifest: &Manifest, dir: &Path) -> Result<String> {
    let json = serde_json::to_string(manifest).map_err(|e| e.to_string())?;
    let mut s = format!("{BUNDLE_FORMAT}\nmanifest {}\n", base64(json.as_bytes()));
    for t in &manifest.tests {
        let bytes = util::read(&dir.join(&t.exe))?;
        if sha256::hex(&bytes) != t.exe_sha256 {
            return Err(format!("{}: changed since the manifest was written; build again", t.exe));
        }
        s.push_str(&format!("file {} {} {}\n", t.exe, t.exe_sha256, base64(&bytes)));
    }
    s.push_str(&format!("end {}\n", manifest.tests.len()));
    Ok(s)
}

/// The bundle's id: the first 12 hex digits of its SHA-256.
pub fn bundle_id(text: &str) -> String {
    sha256::hex(text.as_bytes())[..12].to_string()
}

/// A file name and its bytes.
pub type NamedFile = (String, Vec<u8>);

/// Read a bundle back (what the PowerShell runner does): the manifest's
/// JSON and every file, hashes checked.
pub fn unbundle(text: &str) -> Result<(Manifest, Vec<NamedFile>)> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.is_empty());
    if lines.next() != Some(BUNDLE_FORMAT) {
        return Err(format!("not a {BUNDLE_FORMAT} bundle"));
    }
    let (mut manifest, mut files, mut ended) = (None, Vec::new(), false);
    for line in lines {
        if ended {
            return Err("text after the end line".into());
        }
        let parts: Vec<&str> = line.split(' ').collect();
        match parts.as_slice() {
            ["manifest", b] => {
                let m: Manifest = serde_json::from_slice(&unbase64(b)?).map_err(|e| format!("manifest: {e}"))?;
                manifest = Some(m);
            }
            ["file", name, hash, b] => {
                let bytes = unbase64(b)?;
                if sha256::hex(&bytes) != *hash {
                    return Err(format!("{name}: hash mismatch (damaged in transfer?)"));
                }
                files.push((name.to_string(), bytes));
            }
            ["end", n] => {
                if n.parse::<usize>().ok() != Some(files.len()) {
                    return Err(format!("the end line says {n} files, the bundle has {}", files.len()));
                }
                ended = true;
            }
            _ => return Err(format!("unknown bundle line {:?}", line.chars().take(40).collect::<String>())),
        }
    }
    if !ended {
        return Err("no end line: the bundle is incomplete".into());
    }
    Ok((manifest.ok_or("no manifest line")?, files))
}

/// Cut `text` into parts of at most `limit` bytes at line ends (a line
/// longer than `limit` is a part of its own).
pub fn split_parts(text: &str, limit: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        if !cur.is_empty() && cur.len() + line.len() > limit {
            parts.push(std::mem::take(&mut cur));
        }
        cur.push_str(line);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

// ---------------------------------------------------------------------------
// Results: the same format from Windows and from Poly.

/// The system a result file comes from.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct System {
    /// `windows` or `poly`.
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

fn bits32() -> u8 {
    32
}

/// One test's run.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunResult {
    pub name: String,
    #[serde(default)]
    pub exe_sha256: Option<String>,
    /// `ran`, `timeout`, `error` (could not start it, or the runner
    /// failed) or `unsupported` (Poly cannot run it yet).
    pub status: String,
    /// The process's exit code (`NTSTATUS`-wide).
    #[serde(default)]
    pub exit_code: Option<u32>,
    /// How many low bits of `exit_code` are known (8 from a kernel that
    /// reports only a Linux-style status).
    #[serde(default = "bits32")]
    pub exit_code_bits: u8,
    /// The observation file (`poly-test.out`); `None`: there was none.
    #[serde(default)]
    pub output_b64: Option<String>,
    #[serde(default)]
    pub stdout_b64: Option<String>,
    #[serde(default)]
    pub stderr_b64: Option<String>,
    /// The observation file as text, for people reading the file.
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    /// Windows: whether every run of the test (`-Runs`) gave the same.
    #[serde(default)]
    pub stable: Option<bool>,
    #[serde(default)]
    pub ms: Option<u64>,
}

/// A result file (`poly-winrun/1`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunFile {
    pub format: String,
    pub system: System,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub runner: Option<String>,
    #[serde(default)]
    pub started: Option<String>,
    pub tests: Vec<RunResult>,
}

impl RunFile {
    /// Parse a result file (a UTF-8 byte-order mark, which Windows
    /// PowerShell likes to write, is allowed).
    pub fn parse(bytes: &[u8]) -> Result<RunFile> {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let f: RunFile = serde_json::from_slice(bytes).map_err(|e| format!("not a {RUN_FORMAT} file: {e}"))?;
        if f.format != RUN_FORMAT {
            return Err(format!("format {:?}, expected {RUN_FORMAT}", f.format));
        }
        Ok(f)
    }

    pub fn load(path: &Path) -> Result<RunFile> {
        RunFile::parse(&util::read(path)?).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn test(&self, name: &str) -> Option<&RunResult> {
        self.tests.iter().find(|t| t.name == name)
    }
}

fn decode(field: &Option<String>) -> Result<Option<Vec<u8>>> {
    field.as_deref().map(unbase64).transpose()
}

// ---------------------------------------------------------------------------
// Poly: boot the test and read what the kernel reports.

/// Kernel arguments: the test-run protocol (ADR 0027) plus the
/// observation file, which the kernel reports as standard output
/// (`POLY-OUT: fd=1`) when the program has ended.
pub fn poly_kernel_args() -> String {
    format!("{} poly.test.stdout=/{OUTPUT_FILE}", poly::KERNEL_ARGS)
}

/// A test's run from Poly's serial log. The executable is `/test-binary`
/// in the initramfs root, which Poly's NT personality names
/// `Z:\test-binary`, so the observation file is `/poly-test.out`.
pub fn parse_poly_serial(name: &str, log: &str, exe_sha256: &str, timed_out: bool) -> Result<RunResult> {
    let mut r = RunResult {
        name: name.into(),
        exe_sha256: Some(exe_sha256.into()),
        status: "ran".into(),
        exit_code_bits: 32,
        ..RunResult::default()
    };
    let (mut started, mut ended) = (false, false);
    let (mut output, stdout, stderr) = (Vec::new(), Vec::new(), Vec::new());
    for line in log.lines() {
        let line = line.trim_end_matches('\r');
        let Some(at) = line.find("POLY-") else { continue };
        let Some((tag, rest)) = line[at..].split_once(": ") else { continue };
        let fields: BTreeMap<&str, &str> = rest.split_whitespace().filter_map(|f| f.split_once('=')).collect();
        match tag {
            "POLY-INIT" => {
                if let Some(reason) = rest.strip_prefix("unsupported") {
                    r.status = "unsupported".into();
                    r.error = Some(format!("kernel: {}", reason.trim()));
                    return Ok(r);
                }
                started = true;
            }
            _ if !started => {}
            "POLY-OUT" => {
                let hex = fields.get("hex").ok_or_else(|| format!("POLY-OUT without hex=: {line:?}"))?;
                let bytes = (0..hex.len() / 2)
                    .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
                    .collect::<std::result::Result<Vec<u8>, _>>()
                    .map_err(|_| format!("bad hex in {line:?}"))?;
                // The observation file comes as fd 1 (an NT process has no
                // standard handles on Poly yet, so nothing else does).
                match fields.get("fd").copied() {
                    Some("1") => output.extend_from_slice(&bytes),
                    Some("2") => {}
                    _ => return Err(format!("POLY-OUT without fd 1 or 2: {line:?}")),
                }
            }
            "POLY-EXIT" => {
                let num = |v: &str| -> Result<u64> {
                    match v.strip_prefix("0x") {
                        Some(h) => u64::from_str_radix(h, 16),
                        None => v.parse(),
                    }
                    .map_err(|_| format!("bad number {v:?} in {line:?}"))
                };
                if let Some(s) = fields.get("ntstatus") {
                    r.exit_code = Some(num(s)? as u32);
                } else if let Some(c) = fields.get("code") {
                    r.exit_code = Some(num(c)? as u32);
                    r.exit_code_bits = 8;
                } else if let Some(s) = fields.get("signal") {
                    r.error = Some(format!("ended by signal {s}"));
                } else {
                    return Err(format!("POLY-EXIT without code=, ntstatus= or signal=: {line:?}"));
                }
                ended = true;
            }
            _ => {}
        }
    }
    if !started {
        r.status = "unsupported".into();
        r.error = Some(if timed_out {
            "no \"POLY-INIT:\" line; the boot timed out".into()
        } else {
            "no \"POLY-INIT:\" line: the kernel cannot run programs".into()
        });
        return Ok(r);
    }
    if timed_out {
        r.status = "timeout".into();
    } else if !ended {
        r.status = "error".into();
        r.error = Some("the kernel started the test but did not report its end".into());
    }
    // Poly writes the observation file only if the test made it; an empty
    // report and a missing file look alike, so both are "no file" unless
    // there were bytes.
    r.output = Some(String::from_utf8_lossy(&output).into_owned());
    r.output_b64 = Some(base64(&output));
    r.stdout_b64 = Some(base64(&stdout));
    r.stderr_b64 = Some(base64(&stderr));
    Ok(r)
}

/// Boot each built test on Poly; `work` keeps the initramfs images and
/// serial logs. Tests run `jobs` at a time.
pub fn run_poly(target: &poly::Target, bin: &Path, manifest: &Manifest, work: &Path, jobs: usize) -> Result<RunFile> {
    let started = util::now_utc();
    let results = util::parallel_map(&manifest.tests, jobs, |t| -> Result<RunResult> {
        let bytes = util::read(&bin.join(&t.exe))?;
        let image = poly::initramfs(&bytes)?;
        let (done, log, _) = target.boot_log(&t.name, &image, work, &poly_kernel_args())?;
        let timed_out = done.exit == crate::trace::Exit::Timeout;
        let mut r = parse_poly_serial(&t.name, &log, &t.exe_sha256, timed_out)?;
        r.ms = Some(done.duration_ns / 1_000_000);
        Ok(r)
    });
    let mut tests = Vec::new();
    for (t, r) in manifest.tests.iter().zip(results) {
        tests.push(r.unwrap_or_else(|e| RunResult {
            name: t.name.clone(),
            exe_sha256: Some(t.exe_sha256.clone()),
            status: "error".into(),
            exit_code_bits: 32,
            error: Some(e),
            ..RunResult::default()
        }));
    }
    Ok(RunFile {
        format: RUN_FORMAT.into(),
        system: System {
            name: "poly".into(),
            version: manifest.revision.as_ref().map(|r| format!("rev {}", &r[..12.min(r.len())])),
            build: None,
            arch: Some("x86_64".into()),
        },
        bundle: None,
        runner: Some("cargo xtask pipeline win poly".into()),
        started: Some(started),
        tests,
    })
}

// ---------------------------------------------------------------------------
// The comparison.

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    /// The target cannot run it yet.
    Unsupported,
    /// No result from the target for this test.
    Missing,
    /// The reference is unusable (timed out, failed to start, varies
    /// between runs) or the runs are not of the same executable.
    Error,
}

impl Verdict {
    pub fn word(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
            Verdict::Unsupported => "UNSUPPORTED",
            Verdict::Missing => "MISSING",
            Verdict::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Compared {
    pub name: String,
    pub verdict: Verdict,
    /// Why it is not a pass: the first difference.
    pub detail: String,
}

fn shown(line: &[u8]) -> String {
    let s = String::from_utf8_lossy(line);
    let s = s.trim_end_matches(['\n', '\r']);
    format!("{s:?}")
}

/// The first differing line of two byte strings (1-based), shown.
pub fn first_difference(what: &str, reference: &[u8], target: &[u8]) -> Option<String> {
    if reference == target {
        return None;
    }
    let r: Vec<&[u8]> = reference.split_inclusive(|&b| b == b'\n').collect();
    let t: Vec<&[u8]> = target.split_inclusive(|&b| b == b'\n').collect();
    for i in 0..r.len().max(t.len()) {
        match (r.get(i), t.get(i)) {
            (Some(a), Some(b)) if a == b => continue,
            (Some(a), Some(b)) => {
                return Some(format!("{what} line {}: reference {}, target {}", i + 1, shown(a), shown(b)));
            }
            (Some(a), None) => {
                return Some(format!("{what} line {}: reference {}, target has no more", i + 1, shown(a)));
            }
            (None, Some(b)) => {
                return Some(format!("{what} line {}: reference has no more, target {}", i + 1, shown(b)));
            }
            (None, None) => break,
        }
    }
    Some(format!("{what} differs"))
}

fn hex_code(c: Option<u32>) -> String {
    c.map_or("none".into(), |c| format!("{c:#x}"))
}

/// Compare one test's target run with its reference run.
pub fn compare(reference: &RunResult, target: Option<&RunResult>) -> Result<Compared> {
    let out = |verdict, detail: String| Ok(Compared { name: reference.name.clone(), verdict, detail });
    match reference.status.as_str() {
        "ran" => {}
        "timeout" => return out(Verdict::Error, "the reference run timed out".into()),
        s => {
            let why = reference.error.clone().unwrap_or_default();
            return out(Verdict::Error, format!("the reference did not run ({s}): {why}"));
        }
    }
    if reference.stable == Some(false) {
        return out(Verdict::Error, "the reference differs between its runs: the test is not deterministic".into());
    }
    let Some(target) = target else { return out(Verdict::Missing, "no target result".into()) };
    if let (Some(a), Some(b)) = (&reference.exe_sha256, &target.exe_sha256)
        && a != b
    {
        return out(
            Verdict::Error,
            format!("different executables (reference {}, target {})", &a[..12.min(a.len())], &b[..12.min(b.len())]),
        );
    }
    match target.status.as_str() {
        "ran" => {}
        "unsupported" => return out(Verdict::Unsupported, target.error.clone().unwrap_or_default()),
        "timeout" => return out(Verdict::Fail, "the target run timed out".into()),
        s => {
            return out(
                Verdict::Fail,
                format!("the target did not run ({s}): {}", target.error.clone().unwrap_or_default()),
            );
        }
    }
    let bits = reference.exit_code_bits.min(target.exit_code_bits).min(32);
    let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
    let (rc, tc) = (reference.exit_code.map(|c| c & mask), target.exit_code.map(|c| c & mask));
    let mut problems = Vec::new();
    if rc != tc {
        let how = target.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default();
        problems.push(format!(
            "exit code: reference {}, target {}{how}",
            hex_code(reference.exit_code),
            hex_code(target.exit_code)
        ));
    }
    for (what, r, t) in [
        ("observation", &reference.output_b64, &target.output_b64),
        ("stdout", &reference.stdout_b64, &target.stdout_b64),
        ("stderr", &reference.stderr_b64, &target.stderr_b64),
    ] {
        let (r, t) = (decode(r)?.unwrap_or_default(), decode(t)?.unwrap_or_default());
        if let Some(d) = first_difference(what, &r, &t) {
            problems.push(d);
        }
    }
    if problems.is_empty() { out(Verdict::Pass, String::new()) } else { out(Verdict::Fail, problems.join("; ")) }
}

/// A comparison of two result files (`summary.json`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffSummary {
    pub format: String,
    pub reference: System,
    pub target: System,
    pub totals: BTreeMap<String, usize>,
    pub tests: Vec<Compared>,
}

impl DiffSummary {
    pub fn count(&self, v: Verdict) -> usize {
        self.tests.iter().filter(|t| t.verdict == v).count()
    }
}

fn describe(s: &System) -> String {
    let mut out = s.name.clone();
    for part in [&s.version, &s.build].into_iter().flatten() {
        out.push(' ');
        out.push_str(part);
    }
    out
}

/// Compare every test of `reference` (those in `only`, if any) with
/// `target`.
pub fn diff(reference: &RunFile, target: &RunFile, only: &[String]) -> Result<DiffSummary> {
    let mut tests = Vec::new();
    for r in &reference.tests {
        if !only.is_empty() && !only.contains(&r.name) {
            continue;
        }
        tests.push(compare(r, target.test(&r.name))?);
    }
    let mut totals = BTreeMap::new();
    for t in &tests {
        *totals.entry(t.verdict.word().to_ascii_lowercase()).or_insert(0) += 1;
    }
    Ok(DiffSummary {
        format: DIFF_FORMAT.into(),
        reference: reference.system.clone(),
        target: target.system.clone(),
        totals,
        tests,
    })
}

/// The report: one line per test, the totals.
pub fn report(s: &DiffSummary) -> String {
    let mut out = format!("reference: {}\ntarget:    {}\n\n", describe(&s.reference), describe(&s.target));
    for t in &s.tests {
        if t.detail.is_empty() {
            out.push_str(&format!("{:<12} {}\n", t.verdict.word(), t.name));
        } else {
            out.push_str(&format!("{:<12} {}: {}\n", t.verdict.word(), t.name, t.detail));
        }
    }
    let totals: Vec<String> = s.totals.iter().map(|(k, v)| format!("{v} {k}")).collect();
    out.push_str(&format!("\n{} tests: {}\n", s.tests.len(), totals.join(", ")));
    out
}

// ---------------------------------------------------------------------------
// The visible / held-out split of a Windows entry's tests (ADR 0098).

/// The recorded Windows results of an entry's tests, beside them: for the
/// visible tests `<symbol>.tests/windows.json`, for the held-out ones the
/// same name in the entry's held-out directory (ADR 0075, "Recorded
/// references"). Not a test: `win build` reads only test sources.
pub const RECORDED_RESULTS: &str = "windows.json";

/// What a Windows entry's generated tests must be: test sources only
/// (`.rs`, `.cpp`) with valid names, directly in the directory (support
/// files belong in the repository, `tests/windows/support` and
/// `tests/windows/cxx`).
pub fn check_split_files(files: &[String]) -> Result<()> {
    for f in files {
        let path = Path::new(f);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        if f.contains(['/', '\\']) || !is_test_source(path) || !valid_name(&stem) {
            return Err(format!(
                "{f}: a Windows entry's generated tests are .rs or .cpp files named in lower-case letters, digits \
                 and _, directly in the directory"
            ));
        }
    }
    Ok(())
}

/// Split a Windows result file between the visible tests and the held-out
/// ones (test names). Every test must be in it, have run on Windows and
/// have given the same on every run (a reference that is not deterministic
/// is worth nothing); results of other tests are dropped.
pub fn split_results(results: &RunFile, visible: &[String], held_out: &[String]) -> Result<(RunFile, RunFile)> {
    if results.system.name != "windows" {
        return Err(format!("the results are from {:?}, not from Windows", results.system.name));
    }
    let pick = |names: &[String]| -> Result<RunFile> {
        let mut tests = Vec::new();
        for n in names {
            let r = results.test(n).ok_or_else(|| format!("no Windows result for test {n}"))?;
            if r.status != "ran" {
                return Err(format!("test {n} did not run on Windows ({})", r.status));
            }
            if r.stable == Some(false) {
                return Err(format!("test {n} gave different results on Windows between runs"));
            }
            tests.push(r.clone());
        }
        Ok(RunFile { tests, ..results.clone() })
    };
    Ok((pick(visible)?, pick(held_out)?))
}

/// The test name of a generated file (its stem).
pub fn test_name(file: &str) -> String {
    Path::new(file).file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(name: &str, code: u32, output: &[u8]) -> RunResult {
        RunResult {
            name: name.into(),
            exe_sha256: Some("ab".repeat(32)),
            status: "ran".into(),
            exit_code: Some(code),
            exit_code_bits: 32,
            output_b64: Some(base64(output)),
            stdout_b64: Some(String::new()),
            stderr_b64: Some(String::new()),
            ..RunResult::default()
        }
    }

    #[test]
    fn base64_round_trips() {
        for (raw, enc) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw), enc);
            assert_eq!(unbase64(enc).unwrap(), raw);
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(unbase64(&base64(&all)).unwrap(), all);
        assert!(unbase64("Zg=").is_err());
        assert!(unbase64("Zg==Zg==").is_err());
        assert!(unbase64("Z!==").is_err());
    }

    #[test]
    fn test_names() {
        assert!(valid_name("nt_file_rw"));
        assert!(valid_name("_x2"));
        assert!(!valid_name("2x"));
        assert!(!valid_name("Nt"));
        assert!(!valid_name("a-b"));
        assert!(!valid_name(""));
    }

    #[test]
    fn sources_are_the_corpus_and_extra_dirs() {
        let dir = std::env::temp_dir().join(format!("poly-win-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let corpus = dir.join(CORPUS_DIR);
        util::write(&corpus.join("b_test.rs"), "").unwrap();
        util::write(&corpus.join("support/src/lib.rs"), "").unwrap();
        util::write(&corpus.join("notes.txt"), "").unwrap();
        let extra = dir.join("extra");
        util::write(&extra.join("a_test.rs"), "").unwrap();
        let names: Vec<String> =
            sources(&dir, std::slice::from_ref(&extra)).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["a_test", "b_test"]);
        util::write(&extra.join("b_test.rs"), "").unwrap();
        assert!(sources(&dir, std::slice::from_ref(&extra)).unwrap_err().contains("two tests named b_test"));
        std::fs::remove_file(extra.join("b_test.rs")).unwrap();
        util::write(&extra.join("Bad.rs"), "").unwrap();
        assert!(sources(&dir, &[extra]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn generated_package_lists_every_test() {
        let srcs = [Source { name: "t_one".into(), path: PathBuf::from("/r/tests/windows/t_one.rs") }];
        let toml = cargo_toml(Path::new("/r/tests/windows/support"), &srcs);
        assert!(toml.starts_with("# Generated"));
        assert!(toml.contains("[workspace]\n"));
        assert!(toml.contains("poly-wtest = { path = \"/r/tests/windows/support\" }"));
        assert!(toml.contains("[[bin]]\nname = \"t_one\"\npath = \"/r/tests/windows/t_one.rs\"\n"));
        assert!(toml.contains("panic = \"abort\""));
        assert_eq!(toml_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        let flags = rustflags(Path::new("/r"), Path::new("/s"));
        assert!(flags.contains(&"-Clink-arg=/ENTRY:mainCRTStartup".to_string()));
        assert!(flags.contains(&"-Clink-arg=/Brepro".to_string()));
        assert!(flags.contains(&"--remap-path-prefix=/r=/poly".to_string()));
    }

    /// A PE the repository's own writer makes: an x64 executable with an
    /// import directory for `imports` (PE Format, "The .idata Section")
    /// and the subsystem given.
    fn image(imports: &[(&str, &[&str])], subsystem: u16) -> Vec<u8> {
        use poly_personality_nt::pe_writer::{self, PeImage, PeSection};
        let text =
            PeSection { name: *b".text\0\0\0", data: vec![0xc3], virtual_size: 1, characteristics: pe_writer::TEXT };
        let idata =
            PeSection { name: *b".idata\0\0", data: Vec::new(), virtual_size: 0, characteristics: pe_writer::DATA };
        let mut pe = PeImage {
            image_base: 0x1_4000_0000,
            entry: 0x1000,
            sections: vec![text, idata],
            relocations: Vec::new(),
            fixed: false,
            stack_reserve: 0x10_0000,
            stack_commit: 0x1000,
        };
        let base = pe.section_rva(1);
        // Descriptors, then per DLL: lookup table, address table, names.
        let mut data = vec![0u8; 20 * (imports.len() + 1)];
        for (i, (dll, names)) in imports.iter().enumerate() {
            let table = data.len();
            let slots = 8 * (names.len() + 1);
            data.resize(table + 2 * slots, 0);
            for (j, name) in names.iter().enumerate() {
                let hint_name = data.len() as u64 + u64::from(base);
                data.extend_from_slice(&[0, 0]);
                data.extend_from_slice(name.as_bytes());
                data.push(0);
                data.resize(data.len().next_multiple_of(2), 0);
                for t in [table, table + slots] {
                    data[t + 8 * j..t + 8 * j + 8].copy_from_slice(&hint_name.to_le_bytes());
                }
            }
            let name_rva = data.len() as u32 + base;
            data.extend_from_slice(dll.as_bytes());
            data.push(0);
            let d = 20 * i;
            data[d..d + 4].copy_from_slice(&(table as u32 + base).to_le_bytes());
            data[d + 12..d + 16].copy_from_slice(&name_rva.to_le_bytes());
            data[d + 16..d + 20].copy_from_slice(&((table + slots) as u32 + base).to_le_bytes());
        }
        pe.sections[1].virtual_size = data.len() as u32;
        pe.sections[1].data = data;
        let mut file = pe.write();
        pe_writer::set_directory(&mut file, poly_pe::dir::IMPORT, base, 20 * (imports.len() as u32 + 1));
        let nt = u32::from_le_bytes(file[0x3c..0x40].try_into().unwrap()) as usize;
        // The optional header's Subsystem field (PE Format: offset 68).
        file[nt + 24 + 68..nt + 24 + 70].copy_from_slice(&subsystem.to_le_bytes());
        file
    }

    #[test]
    fn images_import_by_name_from_ntdll_and_kernel32_only() {
        let ok = image(&[("NTDLL.dll", &["NtClose", "NtCreateFile"]), ("kernel32.dll", &["ExitProcess"])], 3);
        let check_image = |b: &[u8]| check_image_with(b, ALLOWED_DLLS);
        let imports = check_image(&ok).unwrap();
        assert_eq!(imports["ntdll.dll"], ["NtClose", "NtCreateFile"]);
        assert_eq!(imports["kernel32.dll"], ["ExitProcess"]);
        let other = image(&[("user32.dll", &["MessageBoxW"])], 3);
        assert!(check_image(&other).unwrap_err().contains("user32.dll"));
        let gui = image(&[("ntdll.dll", &["NtClose"])], 2);
        assert!(check_image(&gui).unwrap_err().contains("subsystem 2"));
        assert!(check_image(b"MZ not really").is_err());
        // C++ tests also import the C++ runtime from msvcrt.dll (ADR 0098).
        let cxx = image(&[("msvcrt.dll", &["_CxxThrowException"]), ("KERNEL32.dll", &["ExitProcess"])], 3);
        assert!(check_image(&cxx).unwrap_err().contains("msvcrt.dll"));
        assert_eq!(check_image_with(&cxx, wincxx::ALLOWED_DLLS).unwrap()["msvcrt.dll"], ["_CxxThrowException"]);
    }

    #[test]
    fn windows_split_checks_files_and_divides_results() {
        assert!(check_split_files(&["a_b.cpp".into(), "c.rs".into()]).is_ok());
        for bad in ["x.h", "sub/a.cpp", "Bad.cpp", "a-b.rs", "windows.json"] {
            assert!(check_split_files(&[bad.into()]).is_err(), "{bad}");
        }
        assert_eq!(test_name("cxx3_x.cpp"), "cxx3_x");

        let mut results = RunFile {
            format: RUN_FORMAT.into(),
            system: System { name: "windows".into(), ..System::default() },
            tests: vec![result("a", 0, b"ok\n"), result("b", 0, b"ok\n"), result("c", 0, b"ok\n")],
            ..RunFile::default()
        };
        let (v, h) = split_results(&results, &["a".into(), "c".into()], &["b".into()]).unwrap();
        assert_eq!(v.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["a", "c"]);
        assert_eq!(h.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(v.system, results.system);
        assert!(split_results(&results, &["d".into()], &[]).unwrap_err().contains("no Windows result for test d"));
        results.tests[1].stable = Some(false);
        assert!(split_results(&results, &[], &["b".into()]).unwrap_err().contains("different results"));
        results.tests[1].stable = Some(true);
        results.tests[1].status = "timeout".into();
        assert!(split_results(&results, &[], &["b".into()]).unwrap_err().contains("did not run"));
        results.system.name = "poly".into();
        assert!(split_results(&results, &[], &[]).unwrap_err().contains("not from Windows"));
    }

    #[test]
    fn cpp_sources_are_tests_too() {
        assert!(is_test_source(Path::new("a/x_y.cpp")) && is_test_source(Path::new("x.rs")));
        assert!(!is_test_source(Path::new("x.h")) && !is_test_source(Path::new("windows.json")));
        let s = Source { name: "x".into(), path: PathBuf::from("d/x.cpp") };
        assert!(s.is_cpp());
        assert!(!Source { name: "x".into(), path: PathBuf::from("d/x.rs") }.is_cpp());
    }

    #[test]
    fn bundles_round_trip_and_split_at_line_ends() {
        let dir = std::env::temp_dir().join(format!("poly-win-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = [("a.exe", vec![1u8, 2, 3]), ("b.exe", vec![0u8; 5000])];
        let mut tests = Vec::new();
        for (name, bytes) in &files {
            util::write(&dir.join(name), bytes).unwrap();
            tests.push(Built {
                name: name.trim_end_matches(".exe").into(),
                source: format!("tests/windows/{name}.rs"),
                source_sha256: "00".repeat(32),
                exe: name.to_string(),
                exe_sha256: sha256::hex(bytes),
                size: bytes.len() as u64,
                imports: BTreeMap::new(),
            });
        }
        let m =
            Manifest { format: BUILD_FORMAT.into(), rustc: "rustc 1.97.0".into(), revision: None, clang: None, tests };
        let text = bundle(&m, &dir).unwrap();
        assert!(text.starts_with("poly-winbundle/1\nmanifest "));
        assert!(text.ends_with("end 2\n"));
        let (back, got) = unbundle(&text).unwrap();
        assert_eq!(back, m);
        assert_eq!(got, files.iter().map(|(n, b)| (n.to_string(), b.clone())).collect::<Vec<_>>());
        assert_eq!(bundle_id(&text).len(), 12);

        let parts = split_parts(&text, 1000);
        assert!(parts.len() >= 3, "the 5000-byte file is a part of its own");
        assert_eq!(parts.concat(), text);
        assert!(parts.iter().all(|p| p.ends_with('\n')));
        // Windows line ends after a copy through an editor are fine.
        assert!(unbundle(&text.replace('\n', "\r\n")).is_ok());

        assert!(unbundle(&text[..text.len() - 6]).unwrap_err().contains("incomplete"));
        let damaged = text.replace("AQID", "AQIE");
        assert!(unbundle(&damaged).unwrap_err().contains("hash mismatch"));
        util::write(&dir.join("a.exe"), [9u8]).unwrap();
        assert!(bundle(&m, &dir).unwrap_err().contains("changed"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn result_files_from_powershell_parse() {
        // What poly-oracle.ps1 writes (ConvertTo-Json, a BOM, nulls, CRLF).
        let text = "\u{feff}{\r\n  \"format\": \"poly-winrun/1\",\r\n  \"system\": { \"name\": \"windows\", \
            \"version\": \"10.0.26200\", \"build\": \"26200.6899\", \"arch\": \"AMD64\" },\r\n  \"bundle\": \
            \"0123456789ab\",\r\n  \"runner\": \"poly-oracle.ps1 1\",\r\n  \"started\": \"2026-10-21T10:00:00Z\",\r\n  \
            \"tests\": [ { \"name\": \"t\", \"exe_sha256\": null, \"status\": \"ran\", \"exit_code\": 3221225477, \
            \"output_b64\": \"b2sgeAo=\", \"stdout_b64\": \"\", \"stderr_b64\": \"\", \"output\": \"ok x\\n\", \
            \"error\": null, \"stable\": true, \"ms\": 31 } ]\r\n}\r\n";
        let f = RunFile::parse(text.as_bytes()).unwrap();
        assert_eq!(f.system.build.as_deref(), Some("26200.6899"));
        let t = f.test("t").unwrap();
        assert_eq!(t.exit_code, Some(0xC000_0005));
        assert_eq!(t.exit_code_bits, 32);
        assert_eq!(unbase64(t.output_b64.as_ref().unwrap()).unwrap(), b"ok x\n");
        assert!(RunFile::parse(b"{\"format\":\"other\",\"system\":{\"name\":\"x\"},\"tests\":[]}").is_err());
    }

    #[test]
    fn poly_serial_logs() {
        let log = "boot\nPOLY-INIT: exec /test-binary\nkernel line\n\
            POLY-OUT: fd=1 hex=6f6b20610a\nPOLY-OUT: fd=1 hex=73756d6d6172790a\n\
            POLY-EXIT: pid=2 code=0 ntstatus=0x0\n";
        let r = parse_poly_serial("t", log, "aa", false).unwrap();
        assert_eq!((r.status.as_str(), r.exit_code, r.exit_code_bits), ("ran", Some(0), 32));
        assert_eq!(r.output.as_deref(), Some("ok a\nsummary\n"));

        let r =
            parse_poly_serial("t", "POLY-INIT: exec /test-binary\nPOLY-EXIT: pid=2 code=53\n", "aa", false).unwrap();
        assert_eq!((r.exit_code, r.exit_code_bits), (Some(53), 8));

        let r =
            parse_poly_serial("t", "POLY-INIT: exec /test-binary\nPOLY-EXIT: pid=2 ntstatus=0xc0000135\n", "", false)
                .unwrap();
        assert_eq!(r.exit_code, Some(0xC000_0135));

        let r = parse_poly_serial("t", "POLY-INIT: unsupported PE: no ntdll\n", "", false).unwrap();
        assert_eq!((r.status.as_str(), r.error.as_deref()), ("unsupported", Some("kernel: PE: no ntdll")));
        assert_eq!(parse_poly_serial("t", "just a boot\n", "", false).unwrap().status, "unsupported");
        assert_eq!(parse_poly_serial("t", "POLY-INIT: exec /x\n", "", true).unwrap().status, "timeout");
        assert_eq!(parse_poly_serial("t", "POLY-INIT: exec /x\n", "", false).unwrap().status, "error");
        assert!(parse_poly_serial("t", "POLY-INIT: exec /x\nPOLY-OUT: fd=1 hex=zz\n", "", false).is_err());
        assert!(poly_kernel_args().ends_with(" poly.test.stdout=/poly-test.out"));
    }

    #[test]
    fn comparisons() {
        let r = result("t", 0, b"ok a\nok b\nsummary 2 passed, 0 failed\n");
        assert_eq!(compare(&r, Some(&r)).unwrap().verdict, Verdict::Pass);

        let mut t = result("t", 1, b"ok a\nnot ok b (got 3)\nsummary 1 passed, 1 failed\n");
        let c = compare(&r, Some(&t)).unwrap();
        assert_eq!(c.verdict, Verdict::Fail);
        assert!(c.detail.contains("exit code: reference 0x0, target 0x1"), "{}", c.detail);
        assert!(
            c.detail.contains("observation line 2: reference \"ok b\", target \"not ok b (got 3)\""),
            "{}",
            c.detail
        );

        // A kernel that reports 8 bits: only those are compared.
        let r5 = result("t", 0xC000_0135, b"");
        t = result("t", 0x35, b"");
        t.exit_code_bits = 8;
        assert_eq!(compare(&r5, Some(&t)).unwrap().verdict, Verdict::Pass);
        t.exit_code = Some(0x36);
        assert_eq!(compare(&r5, Some(&t)).unwrap().verdict, Verdict::Fail);

        t = result("t", 0, b"ok a\n");
        let c = compare(&r, Some(&t)).unwrap();
        assert!(c.detail.contains("observation line 2: reference \"ok b\", target has no more"), "{}", c.detail);

        t.status = "unsupported".into();
        t.error = Some("kernel: no".into());
        assert_eq!(
            compare(&r, Some(&t)).unwrap(),
            Compared { name: "t".into(), verdict: Verdict::Unsupported, detail: "kernel: no".into() }
        );
        assert_eq!(compare(&r, None).unwrap().verdict, Verdict::Missing);

        let mut unstable = r.clone();
        unstable.stable = Some(false);
        assert_eq!(compare(&unstable, Some(&r)).unwrap().verdict, Verdict::Error);
        let mut timeout = r.clone();
        timeout.status = "timeout".into();
        assert_eq!(compare(&timeout, Some(&r)).unwrap().verdict, Verdict::Error);
        let mut other_exe = r.clone();
        other_exe.exe_sha256 = Some("cd".repeat(32));
        assert!(compare(&r, Some(&other_exe)).unwrap().detail.contains("different executables"));
    }

    #[test]
    fn diff_and_report() {
        let reference = RunFile {
            format: RUN_FORMAT.into(),
            system: System { name: "windows".into(), version: Some("10.0.26200".into()), ..System::default() },
            tests: vec![result("a", 0, b"ok\n"), result("b", 0, b"ok\n"), result("c", 0, b"ok\n")],
            ..RunFile::default()
        };
        let mut target = reference.clone();
        target.system = System { name: "poly".into(), ..System::default() };
        target.tests[1].exit_code = Some(1);
        target.tests.pop();
        let s = diff(&reference, &target, &[]).unwrap();
        assert_eq!((s.count(Verdict::Pass), s.count(Verdict::Fail), s.count(Verdict::Missing)), (1, 1, 1));
        let text = report(&s);
        assert!(text.starts_with("reference: windows 10.0.26200\ntarget:    poly\n"));
        assert!(text.contains("PASS         a\n"));
        assert!(text.contains("FAIL         b: exit code"));
        assert!(text.ends_with("3 tests: 1 fail, 1 missing, 1 pass\n"));
        let only = diff(&reference, &target, &["a".into()]).unwrap();
        assert_eq!(only.tests.len(), 1);
    }
}
