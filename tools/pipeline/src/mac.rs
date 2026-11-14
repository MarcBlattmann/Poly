//! The macOS test oracle (ADR 0182): C test programs built with the Mac's
//! own clang for x86_64 and arm64, run on the Mac several times each, and
//! their observations recorded beside the tests as `macos.json`.
//!
//! ```text
//!   tests/macos/*.c, specs/macos/<component>/<symbol>.tests/*.c, held-out dirs
//!     │ mac build: clang -arch x86_64 / -arch arm64, checked with poly-macho
//!     ▼
//!   target/pipeline/macos/bin/<arch>/<test> + manifest.json (poly-macbuild/1)
//!     │ mac run: each test, each architecture, --runs times, fresh scratch dir
//!     ▼
//!   target/pipeline/macos/results/macos-<time>.json (poly-macrun/1)
//!     │ --record: per test directory, merged per test and architecture
//!     ▼
//!   <dir>/macos.json
//! ```
//!
//! Building and running need macOS (Apple clang, the SDK, the kernel under
//! test); elsewhere both commands say so and stop. Parsing, checking and
//! recording are plain Rust and tested on every host.

use crate::sha256;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const MAC_DIR: &str = "target/pipeline/macos";
pub const BUILD_FORMAT: &str = "poly-macbuild/1";
pub const RUN_FORMAT: &str = "poly-macrun/1";
/// The recorded results beside a directory of tests.
pub const RECORDED_RESULTS: &str = "macos.json";
/// The corpus of macOS tests (not entries of the coverage map).
pub const CORPUS_DIR: &str = "tests/macos";
pub const INCLUDE_DIR: &str = "tests/macos/include";
/// Runs per test and architecture unless `--runs` says otherwise.
pub const DEFAULT_RUNS: u32 = 3;

/// The architectures a test is built for, in this order.
pub const ARCHES: [&str; 2] = ["x86_64", "arm64"];

/// What a test may import: nothing. Every call it makes is a raw system
/// call (`tests/macos/include/poly_mtest.h`); the only library it links is
/// libSystem, which macOS needs to start any program.
pub const ALLOWED_DYLIBS: [&str; 1] = ["/usr/lib/libSystem.B.dylib"];

/// The flags every test is compiled with (besides `-arch`, `-isysroot`
/// and the include directory).
pub const CFLAGS: [&str; 9] =
    ["-O1", "-g0", "-std=c11", "-Wall", "-Wextra", "-Werror", "-fno-builtin", "-fno-stack-protector", "-Wl,-S"];

/// Whether this host can build and run macOS tests.
pub fn host_is_macos() -> bool {
    cfg!(target_os = "macos")
}

fn require_macos(what: &str) -> Result {
    if host_is_macos() {
        Ok(())
    } else {
        Err(format!(
            "`mac {what}` needs macOS (Apple clang, the macOS SDK and the kernel under test); this host is {}",
            std::env::consts::OS
        ))
    }
}

// ---------------------------------------------------------------------------
// Finding tests.

/// A test name: lower-case letters, digits and `_`, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// What a macOS entry's generated tests must be (`tests split`): `.c`
/// files with valid names directly in the directory (the support header
/// lives in the repository, `tests/macos/include`).
pub fn check_split_files(files: &[String]) -> Result {
    for f in files {
        let path = Path::new(f);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        if f.contains(['/', '\\']) || path.extension().is_none_or(|e| e != "c") || !valid_name(&stem) {
            return Err(format!(
                "{f}: a macOS entry's generated tests are .c files named in lower-case letters, digits and _, \
                 directly in the directory"
            ));
        }
    }
    Ok(())
}

/// One test program's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub path: PathBuf,
    /// The directory its results are recorded in.
    pub dir: PathBuf,
}

/// The `.c` files directly in `dir`, by name.
pub fn sources_in(dir: &Path) -> Result<Vec<Source>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let path = e.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if !path.is_file() || path.extension().is_none_or(|x| x != "c") {
            continue;
        }
        let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if !valid_name(&name) {
            return Err(format!("{}: test names are lower-case letters, digits and _", path.display()));
        }
        out.push(Source { name, path, dir: dir.to_path_buf() });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The default test directories: the corpus and every macOS entry's
/// visible tests.
pub fn default_dirs(repo: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = vec![repo.join(CORPUS_DIR)];
    let specs = repo.join("specs/macos");
    if specs.is_dir() {
        let mut found = Vec::new();
        for component in std::fs::read_dir(&specs).map_err(|e| format!("{}: {e}", specs.display()))? {
            let component = component.map_err(|e| e.to_string())?.path();
            if !component.is_dir() {
                continue;
            }
            for d in std::fs::read_dir(&component).map_err(|e| format!("{}: {e}", component.display()))? {
                let d = d.map_err(|e| e.to_string())?.path();
                if d.is_dir() && d.extension().is_some_and(|x| x == "tests") {
                    found.push(d);
                }
            }
        }
        found.sort();
        dirs.extend(found);
    }
    Ok(dirs)
}

/// The sources of `dirs`, optionally only the tests named in `only`. A
/// name may appear once.
pub fn collect(dirs: &[PathBuf], only: &[String]) -> Result<Vec<Source>> {
    let mut all: Vec<Source> = Vec::new();
    for d in dirs {
        for s in sources_in(d)? {
            if let Some(other) = all.iter().find(|o| o.name == s.name) {
                return Err(format!(
                    "two tests are called {}: {} and {}",
                    s.name,
                    other.path.display(),
                    s.path.display()
                ));
            }
            all.push(s);
        }
    }
    for n in only {
        if !all.iter().any(|s| &s.name == n) {
            return Err(format!("no macOS test {n}"));
        }
    }
    if !only.is_empty() {
        all.retain(|s| only.contains(&s.name));
    }
    Ok(all)
}

// ---------------------------------------------------------------------------
// Checking an executable.

/// What `mac build` accepts: a thin 64-bit `MH_EXECUTE` for `arch` that
/// loads only [`ALLOWED_DYLIBS`] and imports no symbol. Returns the
/// libraries it loads.
pub fn check_executable(bytes: &[u8], arch: &str) -> Result<Vec<String>> {
    use poly_macho::{CpuKind, FileType, MachO, N_EXT, N_TYPE, N_UNDF};
    let want = match arch {
        "x86_64" => CpuKind::X86_64,
        "arm64" => CpuKind::Arm64,
        _ => return Err(format!("unknown architecture {arch}")),
    };
    let m = MachO::parse(bytes).map_err(|e| format!("not a thin 64-bit Mach-O file: {e:?}"))?;
    let s = m.summary().map_err(|e| format!("{e:?}"))?;
    if s.cpu.kind() != want {
        return Err(format!("built for {:?}, not {arch}", s.cpu.kind()));
    }
    if s.file_type != FileType::Execute {
        return Err(format!("a {:?} file, not an executable", s.file_type));
    }
    let dylibs: Vec<String> = m.dylibs().map(|d| d.name.to_string()).collect();
    if let Some(d) = dylibs.iter().find(|d| !ALLOWED_DYLIBS.contains(&d.as_str())) {
        return Err(format!("loads {d}; a test may load only {}", ALLOWED_DYLIBS.join(", ")));
    }
    if let Some(symtab) = m.symtab().map_err(|e| format!("{e:?}"))? {
        let mut imports = Vec::new();
        for sym in symtab.iter() {
            if sym.n_type & N_EXT != 0 && sym.n_type & N_TYPE == N_UNDF {
                imports.push(String::from_utf8_lossy(symtab.name(&sym).map_err(|e| format!("{e:?}"))?).into_owned());
            }
        }
        if !imports.is_empty() {
            return Err(format!(
                "imports {}; a test makes raw system calls only (poly_mtest.h) and imports nothing",
                imports.join(", ")
            ));
        }
    }
    Ok(dylibs)
}

// ---------------------------------------------------------------------------
// Building.

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltTest {
    pub name: String,
    /// The source, relative to the repository when it is inside it.
    pub source: String,
    pub source_sha256: String,
    /// Architecture -> SHA-256 of the executable.
    pub executables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildManifest {
    pub format: String,
    pub compiler: String,
    pub sdk: String,
    pub flags: Vec<String>,
    pub tests: Vec<BuiltTest>,
}

impl BuildManifest {
    pub fn load(bin_dir: &Path) -> Result<BuildManifest> {
        let m: BuildManifest = util::read_json(&bin_dir.join("manifest.json"))?;
        if m.format != BUILD_FORMAT {
            return Err(format!("{}: format {:?}, expected {BUILD_FORMAT}", bin_dir.display(), m.format));
        }
        Ok(m)
    }
}

fn command_text(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `path` relative to the repository, or as it is when outside it (a
/// held-out directory: recorded only in the held-out store's results).
fn display_path(repo: &Path, path: &Path) -> String {
    let abs = util::absolute(path);
    abs.strip_prefix(repo).map_or_else(|_| abs.display().to_string(), |p| p.display().to_string())
}

/// Build `sources` for both architectures into `out/<arch>/<name>`.
pub fn build(repo: &Path, out: &Path, sources: &[Source]) -> Result<BuildManifest> {
    require_macos("build")?;
    if sources.is_empty() {
        return Err("no macOS tests to build".into());
    }
    let clang = command_text(Command::new("xcrun").args(["--sdk", "macosx", "--find", "clang"]))?;
    let sdk_path = command_text(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]))?;
    let sdk = command_text(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-version"]))?;
    let version = command_text(Command::new(&clang).arg("--version"))?;
    let compiler = version.lines().next().unwrap_or_default().to_string();
    let include = repo.join(INCLUDE_DIR);
    let mut tests = Vec::new();
    for s in sources {
        let mut executables = BTreeMap::new();
        for arch in ARCHES {
            let exe = out.join(arch).join(&s.name);
            util::create_dir(&out.join(arch))?;
            let o = Command::new(&clang)
                .args(["-arch", arch, "-isysroot", &sdk_path])
                .args(CFLAGS)
                .arg(format!("-ffile-prefix-map={}=.", repo.display()))
                .arg("-I")
                .arg(&include)
                .arg(&s.path)
                .arg("-o")
                .arg(&exe)
                .env("ZERO_AR_DATE", "1")
                .output()
                .map_err(|e| format!("{clang}: {e}"))?;
            if !o.status.success() {
                return Err(format!(
                    "{} ({arch}) does not build:\n{}",
                    s.path.display(),
                    String::from_utf8_lossy(&o.stderr).trim_end()
                ));
            }
            let bytes = util::read(&exe)?;
            check_executable(&bytes, arch).map_err(|e| format!("{} ({arch}): {e}", s.path.display()))?;
            executables.insert(arch.to_string(), sha256::hex(&bytes));
        }
        tests.push(BuiltTest {
            name: s.name.clone(),
            source: display_path(repo, &s.path),
            source_sha256: sha256::hex(&util::read(&s.path)?),
            executables,
        });
    }
    let m = BuildManifest {
        format: BUILD_FORMAT.into(),
        compiler,
        sdk: format!("macOS SDK {sdk}"),
        flags: CFLAGS.iter().map(|f| f.to_string()).collect(),
        tests,
    };
    util::write_json(&out.join("manifest.json"), &m)?;
    Ok(m)
}

// ---------------------------------------------------------------------------
// Results.

/// The Mac the results come from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct System {
    /// Always `macos`.
    pub name: String,
    /// `sw_vers -productVersion`, e.g. `27.0`.
    pub product_version: String,
    /// `sw_vers -buildVersion`, e.g. `26A428`.
    pub build: String,
    /// `uname -r`, the Darwin release, e.g. `27.0.0`.
    pub kernel_release: String,
    /// `uname -v`: the kernel's version string (its XNU build).
    pub kernel_version: String,
    /// `uname -m` of the host, `arm64` on Apple silicon.
    pub machine: String,
}

/// Parse `sw_vers` output (`ProductVersion:\t27.0` lines).
pub fn parse_sw_vers(text: &str) -> (String, String) {
    let field = |key: &str| {
        text.lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim() == key)
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    (field("ProductVersion"), field("BuildVersion"))
}

fn this_system() -> Result<System> {
    let (product_version, build) = parse_sw_vers(&command_text(&mut Command::new("sw_vers"))?);
    let uname = |flag: &str| command_text(Command::new("uname").arg(flag));
    Ok(System {
        name: "macos".into(),
        product_version,
        build,
        kernel_release: uname("-r")?,
        kernel_version: uname("-v")?,
        machine: uname("-m")?,
    })
}

/// One run of one executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    /// `ran`, `timeout`, `error` (it did not start) or `unavailable` (this
    /// Mac cannot run the architecture: no Rosetta 2 for x86_64).
    pub status: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Why it did not run.
    pub reason: Option<String>,
}

/// One test on one architecture: the first run, and whether every run
/// gave the same.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchResult {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// Standard output as text (lossless when it is UTF-8, else in
    /// `stdout_base64`).
    #[serde(default)]
    pub stdout: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_base64: Option<String>,
    #[serde(default)]
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub runs: u32,
    pub stable: bool,
}

impl ArchResult {
    /// Whether this is an observation worth recording: the program ran (or
    /// was killed by a signal) the same way every time.
    pub fn recordable(&self) -> bool {
        self.status == "ran" && self.stable
    }
}

fn text_and_base64(bytes: &[u8]) -> (String, Option<String>) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), None),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), Some(crate::windows::base64(bytes))),
    }
}

/// Fold the runs of one test on one architecture into its result.
pub fn summarize(runs: &[Observation], executable_sha256: Option<String>) -> ArchResult {
    let Some(first) = runs.first() else {
        return ArchResult { status: "error".into(), reason: Some("no run".into()), ..ArchResult::default() };
    };
    let stable = runs.iter().all(|r| r == first);
    let (stdout, stdout_base64) = text_and_base64(&first.stdout);
    let (stderr, stderr_base64) = text_and_base64(&first.stderr);
    ArchResult {
        status: first.status.clone(),
        executable_sha256,
        exit_code: first.exit_code,
        signal: first.signal,
        stdout,
        stdout_base64,
        stderr,
        stderr_base64,
        reason: first.reason.clone(),
        runs: runs.len() as u32,
        stable,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub source_sha256: String,
    /// Architecture -> result.
    pub results: BTreeMap<String, ArchResult>,
}

/// A result file (`poly-macrun/1`), and the format of `macos.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFile {
    pub format: String,
    pub system: System,
    pub compiler: String,
    pub sdk: String,
    pub started: String,
    pub tests: Vec<TestResult>,
}

impl RunFile {
    pub fn parse(bytes: &[u8]) -> Result<RunFile> {
        let f: RunFile = serde_json::from_slice(bytes).map_err(|e| format!("not a {RUN_FORMAT} file: {e}"))?;
        if f.format != RUN_FORMAT {
            return Err(format!("format {:?}, expected {RUN_FORMAT}", f.format));
        }
        if f.system.name != "macos" {
            return Err(format!("results from {:?}, not from macOS", f.system.name));
        }
        Ok(f)
    }

    pub fn load(path: &Path) -> Result<RunFile> {
        RunFile::parse(&util::read(path)?).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn test(&self, name: &str) -> Option<&TestResult> {
        self.tests.iter().find(|t| t.name == name)
    }
}

/// Merge `new` results of the tests `names` into an existing recording.
/// Per test and architecture, a recordable new result replaces the old
/// one; anything else (the architecture was unavailable, the test was not
/// stable) keeps what was recorded. A test whose source changed drops its
/// old results: they were of another program. Returns the merged file and
/// the (test, architecture, why) that were not recorded.
pub fn merge_recording(
    old: Option<&RunFile>,
    new: &RunFile,
    names: &[String],
) -> (RunFile, Vec<(String, String, String)>) {
    let mut out = old.cloned().unwrap_or_default();
    out.format = RUN_FORMAT.into();
    out.system = new.system.clone();
    out.compiler = new.compiler.clone();
    out.sdk = new.sdk.clone();
    out.started = new.started.clone();
    let mut skipped = Vec::new();
    for name in names {
        let Some(t) = new.test(name) else {
            skipped.push((name.clone(), "-".into(), "no result".into()));
            continue;
        };
        let pos = out.tests.iter().position(|o| &o.name == name);
        let entry = match pos {
            Some(i) if out.tests[i].source_sha256 == t.source_sha256 => &mut out.tests[i],
            Some(i) => {
                out.tests[i] =
                    TestResult { name: name.clone(), source_sha256: t.source_sha256.clone(), ..Default::default() };
                &mut out.tests[i]
            }
            None => {
                out.tests.push(TestResult {
                    name: name.clone(),
                    source_sha256: t.source_sha256.clone(),
                    ..Default::default()
                });
                out.tests.last_mut().expect("just pushed")
            }
        };
        for (arch, r) in &t.results {
            if r.recordable() {
                entry.results.insert(arch.clone(), r.clone());
            } else {
                let why = if r.status == "ran" { "not stable between runs".to_string() } else { r.status.clone() };
                let why = match &r.reason {
                    Some(reason) => format!("{why}: {reason}"),
                    None => why,
                };
                skipped.push((name.clone(), arch.clone(), why));
            }
        }
    }
    out.tests.retain(|t| !t.results.is_empty());
    out.tests.sort_by(|a, b| a.name.cmp(&b.name));
    (out, skipped)
}

// ---------------------------------------------------------------------------
// Running (macOS only).

/// `EBADARCH` (`<sys/errno.h>`): "Bad CPU type in executable", what
/// posix_spawn and execve say for an x86_64 program without Rosetta 2.
pub const EBADARCH: i32 = 86;

/// Run `exe` once in a fresh, empty `scratch` directory (its current
/// directory), with an empty environment, stdin at end of file and a
/// timeout.
pub fn run_once(exe: &Path, scratch: &Path, timeout: Duration) -> Observation {
    use std::io::Read;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    let fail = |status: &str, reason: String| Observation {
        status: status.into(),
        exit_code: None,
        signal: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        reason: Some(reason),
    };
    if scratch.exists() {
        let _ = std::fs::remove_dir_all(scratch);
    }
    if let Err(e) = util::create_dir(scratch) {
        return fail("error", e);
    }
    let child = Command::new(exe)
        .env_clear()
        .current_dir(scratch)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) if e.raw_os_error() == Some(EBADARCH) => {
            let _ = std::fs::remove_dir_all(scratch);
            return fail("unavailable", format!("{e}; is Rosetta 2 installed?"));
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(scratch);
            return fail("error", format!("could not start: {e}"));
        }
    };
    let mut out_pipe = child.stdout.take().expect("piped");
    let mut err_pipe = child.stderr.take().expect("piped");
    let out_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = out_pipe.read_to_end(&mut v);
        v
    });
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = err_pipe.read_to_end(&mut v);
        v
    });
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    let _ = std::fs::remove_dir_all(scratch);
    match status {
        Some(s) => {
            Observation { status: "ran".into(), exit_code: s.code(), signal: s.signal(), stdout, stderr, reason: None }
        }
        None => Observation {
            status: "timeout".into(),
            exit_code: None,
            signal: None,
            stdout,
            stderr,
            reason: Some(format!("killed after {} s", timeout.as_secs())),
        },
    }
}

/// Run every built test on both architectures `runs` times.
pub fn run(repo: &Path, bin_dir: &Path, runs: u32, timeout: Duration) -> Result<RunFile> {
    require_macos("run")?;
    if runs == 0 {
        return Err("--runs must be at least 1".into());
    }
    let m = BuildManifest::load(bin_dir)?;
    let scratch_root = repo.join(MAC_DIR).join("scratch");
    let mut tests = Vec::new();
    for t in &m.tests {
        let mut results = BTreeMap::new();
        for arch in ARCHES {
            let exe = bin_dir.join(arch).join(&t.name);
            let sha = t.executables.get(arch).cloned();
            if sha.as_deref() != Some(sha256::hex(&util::read(&exe)?).as_str()) {
                return Err(format!("{} is not the executable the manifest names; build again", exe.display()));
            }
            let mut observations = Vec::new();
            for i in 0..runs {
                let o = run_once(&exe, &scratch_root.join(format!("{}-{arch}-{i}", t.name)), timeout);
                let unavailable = o.status == "unavailable";
                observations.push(o);
                if unavailable {
                    break;
                }
            }
            results.insert(arch.to_string(), summarize(&observations, sha));
        }
        tests.push(TestResult { name: t.name.clone(), source_sha256: t.source_sha256.clone(), results });
    }
    Ok(RunFile {
        format: RUN_FORMAT.into(),
        system: this_system()?,
        compiler: m.compiler,
        sdk: m.sdk,
        started: util::now_utc(),
        tests,
    })
}

/// One line per test and architecture, for the terminal.
pub fn describe(r: &ArchResult) -> String {
    let how = match (r.status.as_str(), r.exit_code, r.signal) {
        ("ran", Some(c), _) => format!("exit {c}"),
        ("ran", None, Some(s)) => format!("signal {s}"),
        (s, _, _) => s.to_string(),
    };
    let stable = if r.status == "ran" && !r.stable { ", NOT STABLE" } else { "" };
    let reason = r.reason.as_deref().map_or(String::new(), |s| format!(" ({s})"));
    format!("{how}{stable}{reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(code: i32, out: &str) -> Observation {
        Observation {
            status: "ran".into(),
            exit_code: Some(code),
            signal: None,
            stdout: out.as_bytes().to_vec(),
            stderr: Vec::new(),
            reason: None,
        }
    }

    fn unavailable() -> Observation {
        Observation {
            status: "unavailable".into(),
            exit_code: None,
            signal: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            reason: Some("Bad CPU type in executable (os error 86)".into()),
        }
    }

    fn file(tests: Vec<TestResult>) -> RunFile {
        RunFile {
            format: RUN_FORMAT.into(),
            system: System { name: "macos".into(), product_version: "27.0".into(), ..Default::default() },
            compiler: "clang".into(),
            sdk: "macOS SDK 27.2".into(),
            started: "2026-11-14T00:00:00Z".into(),
            tests,
        }
    }

    fn test(name: &str, src: &str, results: &[(&str, ArchResult)]) -> TestResult {
        TestResult {
            name: name.into(),
            source_sha256: src.into(),
            results: results.iter().map(|(a, r)| (a.to_string(), r.clone())).collect(),
        }
    }

    #[test]
    fn names_and_split_files() {
        assert!(valid_name("bsd_write_basic") && valid_name("a1"));
        for bad in ["", "1a", "Write", "a-b", "_a"] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(check_split_files(&["write_ok.c".into(), "write_ebadf.c".into()]).is_ok());
        for bad in ["write.rs", "sub/write.c", "Write.c", "notes.txt"] {
            assert!(check_split_files(&[bad.into()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn sw_vers_is_parsed() {
        let text = "ProductName:\t\tmacOS\nProductVersion:\t\t27.0\nBuildVersion:\t\t26A428\n";
        assert_eq!(parse_sw_vers(text), ("27.0".into(), "26A428".into()));
        assert_eq!(parse_sw_vers(""), (String::new(), String::new()));
    }

    #[test]
    fn runs_are_summarized() {
        let r = summarize(&[obs(0, "ok a\n"), obs(0, "ok a\n"), obs(0, "ok a\n")], Some("ab".into()));
        assert!(r.stable && r.recordable());
        assert_eq!((r.runs, r.exit_code, r.stdout.as_str()), (3, Some(0), "ok a\n"));
        assert_eq!(describe(&r), "exit 0");
        let r = summarize(&[obs(0, "value x 1\n"), obs(0, "value x 2\n")], None);
        assert!(!r.stable && !r.recordable());
        assert_eq!(describe(&r), "exit 0, NOT STABLE");
        let r = summarize(&[unavailable()], None);
        assert!(!r.recordable());
        assert!(describe(&r).starts_with("unavailable (Bad CPU type"));
        // Output that is not UTF-8 is kept losslessly as base64.
        let mut o = obs(1, "");
        o.stdout = vec![0xff, b'a'];
        let r = summarize(&[o], None);
        assert_eq!(r.stdout_base64.as_deref(), Some("/2E="));
        // A program killed by a signal ran, and is recorded as such.
        let killed = Observation { exit_code: None, signal: Some(9), ..obs(0, "") };
        let r = summarize(&[killed.clone(), killed], None);
        assert!(r.recordable());
        assert_eq!(describe(&r), "signal 9");
    }

    #[test]
    fn recording_merges_per_test_and_architecture() {
        let arm = summarize(&[obs(0, "ok arm\n"), obs(0, "ok arm\n")], Some("a".into()));
        let x86 = summarize(&[obs(0, "ok x86\n"), obs(0, "ok x86\n")], Some("x".into()));
        let none = summarize(&[unavailable()], None);
        // First recording on a Mac without Rosetta: arm64 only.
        let new = file(vec![test("t1", "s1", &[("arm64", arm.clone()), ("x86_64", none.clone())])]);
        let (rec, skipped) = merge_recording(None, &new, &["t1".into()]);
        assert_eq!(rec.tests.len(), 1);
        assert_eq!(rec.tests[0].results.keys().collect::<Vec<_>>(), ["arm64"]);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].1, "x86_64");
        assert!(skipped[0].2.starts_with("unavailable: Bad CPU type"));
        // Later, with Rosetta: x86_64 joins, arm64 stays.
        let new2 = file(vec![test("t1", "s1", &[("x86_64", x86.clone())])]);
        let (rec2, skipped) = merge_recording(Some(&rec), &new2, &["t1".into()]);
        assert!(skipped.is_empty());
        assert_eq!(rec2.tests[0].results.len(), 2);
        assert_eq!(rec2.tests[0].results["arm64"], arm);
        // A changed source drops the old results.
        let new3 = file(vec![test("t1", "s2", &[("x86_64", x86.clone())])]);
        let (rec3, _) = merge_recording(Some(&rec2), &new3, &["t1".into()]);
        assert_eq!(rec3.tests[0].results.keys().collect::<Vec<_>>(), ["x86_64"]);
        // Tests not named stay as they were; an unstable result is not recorded.
        let flaky = summarize(&[obs(0, "1"), obs(0, "2")], None);
        let new4 = file(vec![test("t0", "s0", &[("arm64", flaky)])]);
        let (rec4, skipped) = merge_recording(Some(&rec3), &new4, &["t0".into()]);
        assert_eq!(rec4.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["t1"]);
        assert_eq!(skipped[0].2, "not stable between runs");
        // The recording round-trips through its JSON form.
        let text = serde_json::to_vec(&rec2).unwrap();
        assert_eq!(RunFile::parse(&text).unwrap(), rec2);
        let mut windows = rec2.clone();
        windows.system.name = "windows".into();
        assert!(RunFile::parse(&serde_json::to_vec(&windows).unwrap()).is_err());
    }

    #[test]
    fn sources_are_collected_once() {
        let base = std::env::temp_dir().join(format!("poly-mac-sources-{}", std::process::id()));
        let (a, b) = (base.join("a.tests"), base.join("b.tests"));
        util::write(&a.join("one.c"), "").unwrap();
        util::write(&a.join("two.c"), "").unwrap();
        util::write(&a.join("macos.json"), "{}").unwrap();
        util::write(&b.join("three.c"), "").unwrap();
        let all = collect(&[a.clone(), b.clone()], &[]).unwrap();
        assert_eq!(all.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["one", "two", "three"]);
        assert_eq!(all[2].dir, b);
        let some = collect(&[a.clone(), b.clone()], &["two".into()]).unwrap();
        assert_eq!(some.len(), 1);
        assert!(collect(std::slice::from_ref(&a), &["nope".into()]).is_err());
        util::write(&b.join("one.c"), "").unwrap();
        assert!(collect(&[a, b], &[]).unwrap_err().contains("two tests are called one"));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The executable check, on programs this Mac's clang builds (skipped
    /// elsewhere).
    #[test]
    fn executables_are_checked() {
        if !host_is_macos() || util::which("xcrun").is_none() {
            return eprintln!("skipped: needs macOS with Xcode's command-line tools");
        }
        let base = std::env::temp_dir().join(format!("poly-mac-check-{}", std::process::id()));
        let pure = base.join("pure");
        util::write(&pure.join("pure_one.c"), "#include \"poly_mtest.h\"\nint test_main(void) { return finish(); }\n")
            .unwrap();
        let sources = sources_in(&pure).unwrap();
        let m = build(&crate::repo_root(), &base.join("bin"), &sources).unwrap();
        assert_eq!(m.tests[0].executables.len(), 2);
        // Reproducible: a second build gives the same executables.
        let m2 = build(&crate::repo_root(), &base.join("bin2"), &sources).unwrap();
        assert_eq!(m.tests, m2.tests);
        let arm = util::read(&base.join("bin/arm64/pure_one")).unwrap();
        assert_eq!(check_executable(&arm, "arm64").unwrap(), ["/usr/lib/libSystem.B.dylib"]);
        assert!(check_executable(&arm, "x86_64").unwrap_err().contains("not x86_64"));
        assert!(check_executable(b"not a program", "arm64").is_err());
        // A test that calls into libSystem is refused.
        let impure = base.join("impure");
        util::write(&impure.join("impure_one.c"), "#include <unistd.h>\nint main(void) { return getpid() < 0; }\n")
            .unwrap();
        let err = build(&crate::repo_root(), &base.join("bin3"), &sources_in(&impure).unwrap()).unwrap_err();
        assert!(err.contains("imports _getpid"), "{err}");
        // The arm64 program runs here and passes its (empty) checks.
        let o = run_once(&base.join("bin/arm64/pure_one"), &base.join("scratch"), Duration::from_secs(10));
        assert_eq!((o.status.as_str(), o.exit_code), ("ran", Some(0)), "{o:?}");
        assert_eq!(o.stdout, b"summary 0 passed, 0 failed\n");
        assert!(!base.join("scratch").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
