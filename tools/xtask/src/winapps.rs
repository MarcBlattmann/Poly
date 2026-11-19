//! Any Windows program on Poly, from an unpacked directory (the generic form of
//! `chromewin.rs`, ADR 0155): what it imports that Poly lacks, a run on the
//! console, and a run on Poly's desktop with a window to look at.
//!
//! Nothing of the program is read to learn how Windows behaves (CLEAN_ROOM.md): its PE
//! import tables (names only) say what it needs, and it runs on Poly as a black box.
//!
//! - `cargo xtask win-app imports --dir DIR [--markdown FILE]`: every `.exe` and `.dll` of
//!   the directory against our DLLs, rolled up (as `chrome-win imports`).
//! - `cargo xtask win-app run --dir DIR --exe REL [options]`: boot Poly with the directory
//!   installed as `C:\apps\<name>` and start `REL` there.
//!   - `--mode console` (default): the launcher of `chrome-win` starts it, waits `--seconds`
//!     and prints its standard output and error and how it ended (`LAUNCH-EXIT`). Works on
//!     every host.
//!   - `--mode desktop`: Poly's desktop boots with a desktop file for the program; the
//!     steps (`--steps a,b,c`) start it from the launcher and drive it through QMP (Unix hosts).
//!     Steps: `launch`, `sleep:S`, `wait:TEXT` (a line of the kernel log, 60 s), `shot:NAME`,
//!     `type:TEXT`, `key:QCODE`, `chord:ctrl+s`, `click:X|Y`, `dclick:X|Y`, `rclick:X|Y`,
//!     `drag:X1|Y1|X2|Y2`, `move:X|Y`, `wheel:X|Y|N` (N notches, down when N > 0).
//!     Without steps: `launch`, `sleep:20`, `shot:window`, `sleep:20`, `shot:later`.
//!   - Options: `--name N` (default: the program's file stem), `--args "..."` (the program's
//!     command line after its name), `--then "..."` (repeatable: run the program again with
//!     these arguments after the previous run, in the same boot and on the same disk),
//!     `--dir-cwd REL`, `--seconds S`, `--timeout S` (QEMU's
//!     limit), `--exceptions[=N]`, `--trace`, `--kernel "OPTIONS"` (more kernel options, such as
//!     `poly.nt.waits=10 poly.nt.trail`), `--release`, `--out DIR` (log, summary and
//!     screenshots; default `target/win-app/<name>-out`), `--memory=SIZE`, `--cpus=N`.

use crate::chromewin::{Module, walk};
use crate::winimports::Import;
use crate::{Result, root};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the program is on the disk and as the programs of the disk see it.
const APPS_ON_DISK: &str = "poly/nt/c/apps";
const APPS_DOS: &str = "C:\\apps";

pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("imports") => imports(&args[1..]),
        Some("run") => run(&args[1..]),
        _ => Err("cargo xtask win-app <imports|run> --dir DIR ... (see tools/xtask/src/winapps.rs)".into()),
    }
}

fn is_pe(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    l.ends_with(".exe") || l.ends_with(".dll") || l.ends_with(".pyd") || l.ends_with(".node")
}

fn modules(dir: &Path) -> Result<Vec<Module>> {
    let mut out = Vec::new();
    for (rel, path) in walk(dir)? {
        if !is_pe(&rel) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Ok(imports) = crate::winimports::imports_of(&bytes) {
            out.push(Module { rel, imports });
        }
    }
    Ok(out)
}

/// The exports of the directory's own DLLs, by file name wherever they are (the first one found
/// wins), so imports between the program's files bind.
fn own_dlls(dir: &Path) -> Result<BTreeMap<String, BTreeMap<String, crate::winimports::Target>>> {
    let mut out = BTreeMap::new();
    for (rel, path) in walk(dir)? {
        if !rel.to_ascii_lowercase().ends_with(".dll") {
            continue;
        }
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_ascii_lowercase();
        if out.contains_key(&name) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if let Ok(exports) = crate::winimports::exports_of(&bytes) {
            out.insert(name, exports);
        }
    }
    Ok(out)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Every value of an option that may be given more than once.
fn flags(args: &[String], name: &str) -> Vec<String> {
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].clone()).collect()
}

fn imports(args: &[String]) -> Result {
    let dir = PathBuf::from(flag(args, "--dir").ok_or("--dir DIR is needed")?);
    let mods = modules(&dir)?;
    let mut ours = crate::winimports::load_ours()?;
    let own = own_dlls(&dir)?;
    let own_names: BTreeSet<String> = own.keys().cloned().collect();
    for (name, exports) in own {
        ours.dlls.entry(name).or_insert(exports);
    }
    let all: Vec<Import> = mods.iter().flat_map(|m| m.imports.clone()).collect();
    let cov = crate::winimports::load_coverage(&root(), &ours, &all)?;
    let text = crate::chromewin::rollup(&mods, &ours, &cov, &own_names).markdown();
    println!("{text}");
    if let Some(out) = flag(args, "--markdown") {
        std::fs::write(&out, &text).map_err(|e| format!("{out}: {e}"))?;
    }
    Ok(())
}

/// Whether `path` is a 32-bit (PE32) executable: it runs in a 32-bit process, whose system
/// DLLs are Poly's i686 ones in `SysWOW64` (ADR 0186).
fn is_pe32(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .is_some_and(|b| poly_pe::Pe::parse(&b).is_ok_and(|pe| pe.info().bits() == poly_pe::Bits::Pe32))
}

/// `polystub.dll` for the directory: a stub for every import of its files that Poly cannot bind
/// (`x86`: against the i686 modules, with the 32-bit stubs that end the process when called).
fn polystub(dir: &Path, x86: bool) -> Result<Vec<u8>> {
    let mods = modules(dir)?;
    let mut ours = if x86 { crate::winimports::load_ours_x86()? } else { crate::winimports::load_ours()? };
    for (name, exports) in own_dlls(dir)? {
        ours.dlls.entry(name).or_insert(exports);
    }
    let all: Vec<Import> = mods.iter().flat_map(|m| m.imports.clone()).collect();
    let cov = crate::winimports::load_coverage(&root(), &ours, &all)?;
    let mut wants = Vec::new();
    for m in &mods {
        for i in &m.imports {
            let verdict = crate::winimports::classify(&i.dll, &i.sym, &ours, &cov);
            if matches!(
                verdict,
                crate::winimports::Verdict::MissingDll { .. } | crate::winimports::Verdict::MissingFunction { .. }
            ) {
                wants.push(poly_stubs::Want { dll: i.dll.clone(), symbol: i.sym.to_string(), kind: None });
            }
        }
    }
    let image = if x86 { poly_stubs::build_x86(&wants) } else { poly_stubs::build(&wants) };
    eprintln!("==> polystub.dll: {} stubs, {} KiB", wants.len(), image.len() >> 10);
    Ok(image)
}

/// How a run is made.
struct Opts {
    dir: PathBuf,
    exe: String,
    name: String,
    args: String,
    /// The runs, in order, in one boot: the program and its arguments (`--args`), then the same
    /// program again for each `--then "ARGS"` (or other programs of the directory, for the
    /// boot tests: [`run_sequence`]). Paths are relative to the directory, with `/`.
    runs: Vec<(String, String)>,
    cwd: String,
    desktop: bool,
    seconds: u64,
    timeout: u64,
    exceptions: Option<String>,
    trace: bool,
    /// More kernel options (`--kernel "poly.nt.waits=10 poly.nt.trail"`): the other diagnostics.
    kernel: String,
    release: bool,
    out: PathBuf,
    #[cfg_attr(not(unix), allow(dead_code))]
    steps: Vec<Step>,
    rest: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts> {
    let dir = PathBuf::from(flag(args, "--dir").ok_or("--dir DIR is needed")?);
    let exe = flag(args, "--exe").ok_or("--exe REL (the program inside the directory) is needed")?.replace('\\', "/");
    if exe.contains(' ') {
        return Err("--exe: a path with a space cannot be a desktop file's Exec".into());
    }
    let stem = exe.rsplit('/').next().unwrap_or(&exe);
    let name = flag(args, "--name").unwrap_or_else(|| stem.trim_end_matches(".exe").to_ascii_lowercase());
    let name: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let mode = flag(args, "--mode").unwrap_or_else(|| "console".into());
    if mode != "console" && mode != "desktop" {
        return Err(format!("--mode {mode}: console or desktop"));
    }
    let num = |n: &str, d: u64| -> Result<u64> {
        flag(args, n).map_or(Ok(d), |v| v.parse().map_err(|e| format!("{n}: {e}")))
    };
    let steps = match flag(args, "--steps") {
        Some(s) => split_steps(&s).iter().map(|p| Step::parse(p)).collect::<Result<Vec<_>>>()?,
        None => ["launch", "sleep:20", "shot:window", "sleep:20", "shot:later"]
            .iter()
            .map(|s| Step::parse(s))
            .collect::<Result<Vec<_>>>()?,
    };
    let rest: Vec<String> =
        args.iter().filter(|a| a.starts_with("--memory=") || a.starts_with("--cpus=")).cloned().collect();
    Ok(Opts {
        out: flag(args, "--out")
            .map_or_else(|| root().join("target/win-app").join(format!("{name}-out")), PathBuf::from),
        dir,
        runs: core::iter::once(flag(args, "--args").unwrap_or_default())
            .chain(flags(args, "--then"))
            .map(|a| (exe.clone(), a))
            .collect(),
        exe,
        args: flag(args, "--args").unwrap_or_default(),
        cwd: flag(args, "--dir-cwd").unwrap_or_default(),
        desktop: mode == "desktop",
        seconds: num("--seconds", 60)?,
        timeout: num("--timeout", 900)?,
        exceptions: args.iter().find_map(|a| {
            if a == "--exceptions" {
                Some(String::new())
            } else {
                a.strip_prefix("--exceptions").filter(|r| r.starts_with('=')).map(str::to_string)
            }
        }),
        trace: args.iter().any(|a| a == "--trace"),
        kernel: flag(args, "--kernel").unwrap_or_default(),
        release: args.iter().any(|a| a == "--release"),
        name,
        steps,
        rest,
    })
}

/// The disk: the program's directory as `C:\apps\<name>`, the launcher, the stage file
/// (`stage1.cmd`), `polystub.dll`.
fn disk(o: &Opts) -> Result<PathBuf> {
    let work = root().join("target/win-app");
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let image = work.join(format!("{}.img", o.name));
    let built = crate::winapp::build(&[crate::chromewin::LAUNCHER])?;
    let launcher = built.into_iter().next().map(|(_, b)| b).ok_or("the launcher was not built")?;
    let x86 = is_pe32(&o.dir.join(&o.exe));
    let stubs = polystub(&o.dir, x86)?;
    let exe_dos = format!("{APPS_DOS}\\{}\\{}", o.name, o.exe.replace('/', "\\"));
    let cwd = if o.cwd.is_empty() {
        exe_dos.rsplit_once('\\').map_or(APPS_DOS.to_string(), |p| p.0.to_string())
    } else {
        o.cwd.clone()
    };
    // One stage per run; each but the last asks the launcher for the next one (line 7).
    let stages: Vec<String> = o
        .runs
        .iter()
        .enumerate()
        .map(|(i, (exe, args))| {
            let exe_dos = format!("{APPS_DOS}\\{}\\{}", o.name, exe.replace('/', "\\"));
            let cmdline = format!("\"{exe_dos}\" {args}").trim_end().to_string();
            let next = if i + 1 < o.runs.len() { "next" } else { "" };
            format!("{exe_dos}\n{cmdline}\n{cwd}\n{}\n\n\n{next}\n", o.seconds)
        })
        .collect();
    let mut tree = crate::rootdisk::RootTree::default();
    for d in ["poly", "poly/nt", "poly/nt/registry", "poly/nt/c", APPS_ON_DISK] {
        tree.dir(d, 0o755);
    }
    for (rel, path) in walk(&o.dir)? {
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let low = rel.to_ascii_lowercase();
        let mode = if low.ends_with(".exe") || low.ends_with(".dll") { 0o755 } else { 0o644 };
        tree.host_file(&format!("{APPS_ON_DISK}/{}/{rel}", o.name), mode, &path, crate::hostfs::mtime(&meta));
    }
    for (i, stage) in stages.iter().enumerate() {
        tree.file(&format!("poly/nt/c/poly-test/stage{}.cmd", i + 1), 0o644, stage.as_bytes());
    }
    tree.file(&format!("poly/nt/c/poly-test/{}.exe", crate::chromewin::LAUNCHER), 0o755, &launcher);
    // A 32-bit program's loader searches SysWOW64, its System32 (ADR 0186).
    let system = if x86 { "poly/nt/c/Windows/SysWOW64" } else { crate::chromewin::SYSTEM32_ON_DISK };
    tree.file(&format!("{system}/{}", poly_stubs::STUB_DLL), 0o755, &stubs);
    // Poly's fonts in `C:\Windows\Fonts` (ADR 0181), as `chrome-win`'s disk has them: without a
    // face, Chrome's UI font set-up starts over until the browser's stack is full (ADR 0155
    // addenda "step 9" and "S3 hunt").
    crate::fonts::fetch_dejavu()?;
    crate::fonts::add_to_root(&mut tree)?;
    let started = Instant::now();
    crate::rootdisk::write_image_poly_ext2(&tree, &image)?;
    eprintln!("==> disk {} ({} entries, {:.0?})", image.display(), tree.entries.len(), started.elapsed());
    Ok(image)
}

fn kernel_arguments(o: &Opts) -> String {
    let mut more = String::new();
    if o.trace {
        more.push_str(" poly.nt.trace");
    }
    if let Some(v) = &o.exceptions {
        more.push_str(" poly.nt.exceptions");
        more.push_str(v);
    }
    for option in o.kernel.split_whitespace() {
        more.push(' ');
        more.push_str(option);
    }
    more
}

/// Memory of a run: big programs (Electron, Firefox, LibreOffice) need room.
const MEMORY: &str = "4G";
const CPUS: u32 = 4;

fn run(args: &[String]) -> Result {
    let o = parse(args)?;
    if !o.dir.join(&o.exe).is_file() {
        return Err(format!("{}: not in the directory", o.dir.join(&o.exe).display()));
    }
    std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
    let kernel = crate::build_kernel(o.release)?;
    let image = disk(&o)?;
    let log = o.out.join(format!("{}.serial.log", o.name));
    let _ = std::fs::remove_file(&log);
    let res = crate::resources::Resources::from_args(&o.rest)?;
    eprintln!("{}", res.describe(MEMORY, CPUS));
    let more = kernel_arguments(&o);
    let text = if o.desktop {
        run_desktop(&o, &kernel, &image, &log, &res, &more)?
    } else {
        run_console(&o, &kernel, &image, &log, &res, &more)?
    };
    let text = clean(&text);
    let mut summary = crate::chromewin::Summary::parse(&text, "");
    summary.stubs.sort();
    let report = report(&o, &summary, &text);
    let path = o.out.join(format!("{}.summary.txt", o.name));
    std::fs::write(&path, &report).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("{report}");
    eprintln!("==> log {}, summary {}", log.display(), path.display());
    Ok(())
}

/// Boot `kernel` with the program directory `dir` as `C:\apps\<name>` and run `runs` (a
/// program of the directory, relative with `/`, and its arguments) one after another in `cwd`
/// (a DOS path), each for at most `seconds`: the serial log, cleaned, also written to `log`. For
/// boot tests of downloaded apps (`gitwin.rs`).
pub fn run_sequence(
    kernel: &Path,
    dir: &Path,
    name: &str,
    cwd: &str,
    runs: &[(&str, &str)],
    seconds: u64,
    log: &Path,
) -> Result<String> {
    let (exe, args) = *runs.first().ok_or("nothing to run")?;
    let o = Opts {
        dir: dir.to_path_buf(),
        exe: exe.to_string(),
        name: name.to_string(),
        args: args.to_string(),
        runs: runs.iter().map(|&(e, a)| (e.to_string(), a.to_string())).collect(),
        cwd: cwd.to_string(),
        desktop: false,
        seconds,
        timeout: 120 + seconds * runs.len() as u64,
        exceptions: Some(String::new()),
        trace: false,
        kernel: String::new(),
        release: false,
        out: log.parent().map_or_else(|| root().join("target"), Path::to_path_buf),
        steps: Vec::new(),
        rest: Vec::new(),
    };
    let image = disk(&o)?;
    let _ = std::fs::remove_file(log);
    let res = crate::resources::Resources::from_args(&[])?;
    let text = run_console(&o, kernel, &image, log, &res, &kernel_arguments(&o))?;
    Ok(clean(&text))
}

/// The serial log without the terminal's escape sequences (a cursor move starts a new line).
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut it = text.chars();
    while let Some(c) = it.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        let mut rest = it.clone();
        if rest.next() != Some('[') {
            continue;
        }
        let (mut params, mut last) = (String::new(), ' ');
        for n in rest.by_ref() {
            if n.is_ascii_alphabetic() {
                last = n;
                break;
            }
            params.push(n);
        }
        it = rest;
        if last == 'H' && !params.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out.replace('\r', "")
}

/// What the program wrote to its standard output or error, as the launcher printed it.
fn program_output(text: &str, which: &str) -> Vec<String> {
    let (begin, end) = (format!("LAUNCH-BEGIN {which}"), format!("LAUNCH-END {which}"));
    let mut inside = false;
    let mut out = Vec::new();
    for l in text.lines() {
        if l.contains(&end) {
            inside = false;
        }
        if inside && !l.trim().is_empty() {
            out.push(l.trim_end().to_string());
        }
        if l.contains(&begin) {
            inside = true;
        }
    }
    out
}

fn report(o: &Opts, s: &crate::chromewin::Summary, text: &str) -> String {
    use std::fmt::Write as _;
    let mut r = String::new();
    let _ = writeln!(r, "program: {}\\{}\\{} {}", APPS_DOS, o.name, o.exe, o.args);
    let _ = writeln!(r, "launcher: {}", s.exit.as_deref().unwrap_or("(no LAUNCH-EXIT line)"));
    for (i, (exe, args)) in o.runs.iter().enumerate().skip(1) {
        let _ = writeln!(r, "then {}: {exe} {args}", i + 1);
    }
    if o.runs.len() > 1 {
        for l in text.lines().filter(|l| l.contains("LAUNCH-EXIT") && !l.contains(crate::console::RESULT_FRAME)) {
            let _ = writeln!(r, "  {}", l.trim());
        }
    }
    for which in ["stdout", "stderr"] {
        let lines = program_output(text, which);
        let _ = writeln!(r, "{which} ({} lines):", lines.len());
        for l in lines.iter().take(40) {
            let _ = writeln!(r, "  | {l}");
        }
    }
    let _ = writeln!(r, "loader reasons ({}):", s.loader.len());
    for l in s.loader.iter().take(40) {
        let _ = writeln!(r, "  {l}");
    }
    let _ = writeln!(r, "stub calls reached ({}):", s.stubs.len());
    for l in s.stubs.iter().take(80) {
        let _ = writeln!(r, "  {l}");
    }
    let _ = writeln!(r, "faults and unimplemented ({}):", s.faults.len());
    for l in s.faults.iter().take(40) {
        let _ = writeln!(r, "  {l}");
    }
    let exc: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("nt: ") && l.contains("exception") && !l.contains("poly.nt.exceptions"))
        .take(20)
        .collect();
    let comp: Vec<&str> = text
        .lines()
        .filter(|l| {
            l.contains("poly-compositor: ") && (l.contains("exited") || l.contains("launch ") || l.contains("window"))
        })
        .take(20)
        .collect();
    let _ = writeln!(r, "compositor lines about the program ({}):", comp.len());
    for l in comp {
        let _ = writeln!(r, "  {}", l.trim_end());
    }
    let _ = writeln!(r, "exception lines (first {}):", exc.len());
    for l in exc {
        let _ = writeln!(r, "  {}", l.trim_end());
    }
    r
}

/// A console run: headless, the launcher is the first program.
fn run_console(
    o: &Opts,
    kernel: &Path,
    image: &Path,
    log: &Path,
    res: &crate::resources::Resources,
    more: &str,
) -> Result<String> {
    let launcher = crate::chromewin::LAUNCHER;
    let append = format!(
        "poly.mount=virtio0:/mnt/root poly.init=/mnt/root/poly/nt/c/poly-test/{launcher}.exe poly.init.arg=1 \
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
    cmd.args(crate::rootdisk::drive_args(image));
    cmd.args(crate::net::nic_args(None, false));
    cmd.args(["-display", "none"]).stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    let (started, limit) = (Instant::now(), Duration::from_secs(o.timeout));
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            eprintln!("==> QEMU stopped after {} s", o.timeout);
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
}

/// The desktop file that starts the program. `%` begins a field code in `Exec` (the Desktop
/// Entry Specification, "The Exec key"); a literal one is written `%%`, so a URL's `%20`
/// reaches the program as it was given.
#[cfg_attr(not(unix), allow(dead_code))]
fn desktop_file(name: &str, exe: &str, args: &str) -> String {
    let a = args.replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName=Winapp {name}\nComment=Windows program under test\nExec={exe} {a}\nKeywords=winapp;{name};\n"
    )
}

/// A desktop run: the compositor is init, a desktop file starts the program, QMP drives it.
fn run_desktop(
    o: &Opts,
    kernel: &Path,
    image: &Path,
    log: &Path,
    res: &crate::resources::Resources,
    more: &str,
) -> Result<String> {
    let (text, stopped) = run_desktop_checked(o, kernel, image, log, res, more)?;
    if let Some(e) = stopped {
        eprintln!("==> the steps stopped: {e}");
    }
    Ok(text)
}

/// [`run_desktop`] that also says why the steps stopped, if they did (a step that failed, such
/// as a `pixel` check).
#[cfg(unix)]
fn run_desktop_checked(
    o: &Opts,
    kernel: &Path,
    image: &Path,
    log: &Path,
    res: &crate::resources::Resources,
    more: &str,
) -> Result<(String, Option<String>)> {
    let dos = format!("/mnt/root/{APPS_ON_DISK}/{}/{}", o.name, o.exe);
    let file = desktop_file(&o.name, &dos, &o.args);
    let extra = vec![(format!("usr/share/applications/winapp-{}.desktop", o.name), 0o644, file.into_bytes())];
    let initrd = crate::desktop::image_with("poly-win-app-initramfs.cpio", false, extra)?;
    let spec = DriveSpec {
        out: o.out.clone(),
        name: o.name.clone(),
        query: format!("winapp {}", o.name),
        steps: o.steps.clone(),
        timeout: o.timeout,
    };
    boot_and_drive(kernel, image, &initrd, "virtio0:/mnt/root", res, more, log, &spec)
}

/// What a scripted desktop run does once the desktop is up.
#[cfg(unix)]
struct DriveSpec {
    /// Where the screenshots go.
    out: PathBuf,
    /// Their names, and the `launch` step's window.
    name: String,
    /// What the `launch` step types into the launcher before Enter.
    query: String,
    steps: Vec<Step>,
    timeout: u64,
}

/// Boot the desktop from `initrd` with `image` as its root disk (`mount` is the `poly.mount=`
/// value), serial output into `log`, and run the steps of `spec` through QMP.
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn boot_and_drive(
    kernel: &Path,
    image: &Path,
    initrd: &Path,
    mount: &str,
    res: &crate::resources::Resources,
    more: &str,
    log: &Path,
    spec: &DriveSpec,
) -> Result<(String, Option<String>)> {
    let socket = std::env::temp_dir().join(format!("poly-win-app-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let append = crate::desktop::append(&format!("poly.mount={mount} console=ttyS0 poly.net=qemu{more}"));
    let mut cmd = crate::qemu_with(
        kernel,
        &res.memory(MEMORY),
        res.cpus_or(CPUS),
        &append,
        crate::Devices::None,
        &format!("file:{}", log.display()),
    );
    cmd.args(crate::rootdisk::drive_args(image));
    cmd.args(crate::net::nic_args(None, false));
    cmd.arg("-initrd").arg(initrd).args(crate::desktop::devices());
    cmd.args(["-display", "none", "-qmp", &format!("unix:{},server=on,wait=off", socket.display())])
        .stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    let result = drive(spec, &socket, log);
    let _ = std::fs::remove_file(&socket);
    let _ = child.kill();
    let _ = child.wait();
    let text = std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    Ok((text, result.err()))
}

/// A scripted run on the desktop with a disk and a launcher entry that are not `win-app`'s: boot
/// `kernel` with `image` as the root disk (`mount`: its `poly.mount=` value) and `entry` (the
/// files, path below the initramfs's root, mode and contents) in the initramfs, then type `query`
/// into the launcher at the `launch` step and run `steps`. The cleaned serial log (also written to
/// `log`) and the reason the steps stopped, if a step failed. For the boot check of what
/// `cargo xtask start` boots (`chrometry.rs`).
#[cfg(unix)]
pub fn desktop_entry_check(
    kernel: &Path,
    image: &Path,
    entry: Vec<(String, u16, Vec<u8>)>,
    mount: &str,
    query: &str,
    steps: &str,
    log: &Path,
) -> Result<(String, Option<String>)> {
    let steps = split_steps(steps).iter().map(|p| Step::parse(p)).collect::<Result<Vec<_>>>()?;
    let out = log.parent().map_or_else(|| root().join("target"), Path::to_path_buf);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let _ = std::fs::remove_file(log);
    let initrd = crate::desktop::image_with("poly-chrome-try-initramfs.cpio", false, entry)?;
    let res = crate::resources::Resources::from_args(&[])?;
    let spec = DriveSpec { out, name: "chrome-try".into(), query: query.to_string(), steps, timeout: 900 };
    let (text, stopped) = boot_and_drive(kernel, image, &initrd, mount, &res, "", log, &spec)?;
    Ok((clean(&text), stopped))
}

#[cfg(not(unix))]
fn run_desktop_checked(
    _: &Opts,
    _: &Path,
    _: &Path,
    _: &Path,
    _: &crate::resources::Resources,
    _: &str,
) -> Result<(String, Option<String>)> {
    Err("--mode desktop needs a Unix host (QMP over a Unix socket); run it with polyd".into())
}

/// [`desktop_entry_check`] needs QMP over a Unix socket.
#[cfg(not(unix))]
pub fn desktop_entry_check(
    _: &Path,
    _: &Path,
    _: Vec<(String, u16, Vec<u8>)>,
    _: &str,
    _: &str,
    _: &str,
    _: &Path,
) -> Result<(String, Option<String>)> {
    Err("a scripted desktop run needs a Unix host (QMP over a Unix socket); run it with polyd".into())
}

/// Boot `kernel` on the desktop with the program directory `dir` as `C:pps\<name>`, start
/// `exe` with `args` from the launcher and run `steps` (the steps of `--steps`, comma separated,
/// ending in a `pixel:X|Y|RRGGBB` check, for instance): the cleaned serial log (also written to
/// `log`) and the reason the steps stopped, if a step failed. For boot tests of a program's window
/// (`chromewin.rs`, stage 3).
pub fn desktop_check(
    kernel: &Path,
    dir: &Path,
    exe: &str,
    name: &str,
    args: &str,
    steps: &str,
    log: &Path,
) -> Result<(String, Option<String>)> {
    let steps = split_steps(steps).iter().map(|p| Step::parse(p)).collect::<Result<Vec<_>>>()?;
    let out = log.parent().map_or_else(|| root().join("target"), Path::to_path_buf);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let o = Opts {
        dir: dir.to_path_buf(),
        exe: exe.to_string(),
        name: name.to_string(),
        args: args.to_string(),
        runs: vec![(exe.to_string(), args.to_string())],
        cwd: String::new(),
        desktop: true,
        seconds: 60,
        timeout: 900,
        exceptions: Some(String::new()),
        trace: false,
        kernel: String::new(),
        release: false,
        out,
        steps,
        rest: Vec::new(),
    };
    let image = disk(&o)?;
    let _ = std::fs::remove_file(log);
    let res = crate::resources::Resources::from_args(&[])?;
    let (text, stopped) = run_desktop_checked(&o, kernel, &image, log, &res, &kernel_arguments(&o))?;
    Ok((clean(&text), stopped))
}

/// The steps of `--steps`, comma separated. A piece that does not start a step continues the text
/// of the `type` or `wtype` step before it, comma and all, so a typed URL may have commas
/// (`type:data:text/html,<b>x</b>`).
fn split_steps(s: &str) -> Vec<String> {
    const KINDS: &[&str] = &[
        "launch", "sleep", "wait", "shot", "pixel", "near", "type", "wtype", "key", "chord", "click", "dclick",
        "rclick", "drag", "wheel", "move",
    ];
    let mut out: Vec<String> = Vec::new();
    for piece in s.split(',') {
        let kind = piece.split_once(':').map_or(piece, |p| p.0);
        let typing = out.last().is_some_and(|l| l.starts_with("type:") || l.starts_with("wtype:"));
        if typing && !KINDS.contains(&kind) {
            if let Some(last) = out.last_mut() {
                last.push(',');
                last.push_str(piece);
            }
        } else if !piece.is_empty() {
            out.push(piece.to_string());
        }
    }
    out
}

/// Whether `steps` (as `--steps` takes them) are all steps, for the boot tests' own checks.
#[cfg(test)]
pub fn steps_parse(steps: &str) -> bool {
    split_steps(steps).iter().all(|p| Step::parse(p).is_ok())
}

/// One step of a desktop run.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Step {
    Launch,
    Sleep(u64),
    Wait(String),
    Shot(String),
    /// The screen's pixel at X|Y must be this colour (RRGGBB): the step fails otherwise.
    Pixel(i32, i32, [u8; 3]),
    /// The screen's pixel at X|Y must be this colour (RRGGBB) within T in each channel
    /// (`near:X|Y|RRGGBB|T`): the page seen through a popup's soft shadow, which a popup composed
    /// without its alpha shows near black (ADR 0225).
    Near(i32, i32, [u8; 3], u8),
    Type(String),
    /// Type for a Windows program: the keys of win32k's keyboard layout
    /// (German, `poly_win32k::kbd`), where `type` uses the US positions.
    WType(String),
    Key(String),
    Chord(Vec<String>),
    Click(i32, i32),
    DClick(i32, i32),
    RClick(i32, i32),
    /// Press the left button at the first point, move to the second in steps, release.
    Drag(i32, i32, i32, i32),
    /// Turn the wheel at X|Y by N notches: down (towards the user) when N > 0, up when N < 0.
    Wheel(i32, i32, i32),
    /// Move the pointer to X|Y without a button.
    Move(i32, i32),
}

impl Step {
    fn parse(s: &str) -> Result<Step> {
        let (kind, arg) = s.split_once(':').unwrap_or((s, ""));
        let xy = || -> Result<(i32, i32)> {
            let (x, y) = arg.split_once('|').or_else(|| arg.split_once('/')).ok_or(format!("step {s:?}: X|Y"))?;
            Ok((
                x.trim().parse().map_err(|_| format!("step {s:?}: bad X"))?,
                y.trim().parse().map_err(|_| format!("step {s:?}: bad Y"))?,
            ))
        };
        Ok(match kind {
            "launch" => Step::Launch,
            "sleep" => Step::Sleep(arg.parse().map_err(|_| format!("step {s:?}: seconds"))?),
            "wait" => Step::Wait(arg.into()),
            "shot" => Step::Shot(arg.into()),
            "near" => {
                let v: Vec<&str> = arg.split('|').collect();
                let [x, y, rgb, t] = v[..] else { return Err(format!("step {s:?}: X|Y|RRGGBB|T")) };
                let colour = u32::from_str_radix(rgb.trim(), 16).ok().filter(|_| rgb.trim().len() == 6);
                let (Ok(x), Ok(y), Some(c), Ok(t)) = (x.trim().parse(), y.trim().parse(), colour, t.trim().parse())
                else {
                    return Err(format!("step {s:?}: X|Y|RRGGBB|T"));
                };
                Step::Near(x, y, [(c >> 16) as u8, (c >> 8) as u8, c as u8], t)
            }
            "pixel" => {
                let v: Vec<&str> = arg.split('|').collect();
                let [x, y, rgb] = v[..] else { return Err(format!("step {s:?}: X|Y|RRGGBB")) };
                let colour = u32::from_str_radix(rgb.trim(), 16).ok().filter(|_| rgb.trim().len() == 6);
                let (Ok(x), Ok(y), Some(c)) = (x.trim().parse(), y.trim().parse(), colour) else {
                    return Err(format!("step {s:?}: X|Y|RRGGBB"));
                };
                Step::Pixel(x, y, [(c >> 16) as u8, (c >> 8) as u8, c as u8])
            }
            "type" => Step::Type(arg.into()),
            "wtype" => Step::WType(arg.into()),
            "key" => Step::Key(arg.into()),
            "chord" => Step::Chord(arg.split('+').map(str::to_string).collect()),
            "click" => {
                let (x, y) = xy()?;
                Step::Click(x, y)
            }
            "dclick" => {
                let (x, y) = xy()?;
                Step::DClick(x, y)
            }
            "rclick" => {
                let (x, y) = xy()?;
                Step::RClick(x, y)
            }
            "wheel" => {
                let v: Vec<i32> = arg
                    .split('|')
                    .map(|p| p.trim().parse())
                    .collect::<core::result::Result<_, _>>()
                    .map_err(|_| format!("step {s:?}: X|Y|N"))?;
                let [x, y, n] = v[..] else { return Err(format!("step {s:?}: X|Y|N")) };
                Step::Wheel(x, y, n)
            }
            "move" => {
                let (x, y) = xy()?;
                Step::Move(x, y)
            }
            "drag" => {
                let v: Vec<i32> = arg
                    .split('|')
                    .map(|p| p.trim().parse())
                    .collect::<core::result::Result<_, _>>()
                    .map_err(|_| format!("step {s:?}: X1|Y1|X2|Y2"))?;
                let [x1, y1, x2, y2] = v[..] else { return Err(format!("step {s:?}: X1|Y1|X2|Y2")) };
                Step::Drag(x1, y1, x2, y2)
            }
            _ => return Err(format!("step {s:?}: no such step (click takes X|Y because steps are comma separated)")),
        })
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
/// QEMU's key name (and whether Shift is held) for a character on the US layout.
fn qcode(c: char) -> Option<(String, bool)> {
    let plain = |s: &str| Some((s.to_string(), false));
    let shifted = |s: &str| Some((s.to_string(), true));
    match c {
        'a'..='z' | '0'..='9' => plain(&c.to_string()),
        'A'..='Z' => shifted(&c.to_ascii_lowercase().to_string()),
        ' ' => plain("spc"),
        '/' => plain("slash"),
        '.' => plain("dot"),
        ',' => plain("comma"),
        '-' => plain("minus"),
        ';' => plain("semicolon"),
        '\'' => plain("apostrophe"),
        '\\' => plain("backslash"),
        '=' => plain("equal"),
        ':' => shifted("semicolon"),
        '_' => shifted("minus"),
        '!' => shifted("1"),
        '(' => shifted("9"),
        ')' => shifted("0"),
        '>' => shifted("dot"),
        '<' => shifted("comma"),
        '?' => shifted("slash"),
        '&' => shifted("7"),
        '#' => shifted("3"),
        '%' => shifted("5"),
        '@' => shifted("2"),
        '+' => shifted("equal"),
        '*' => shifted("8"),
        '"' => shifted("apostrophe"),
        _ => None,
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
/// The keys (QEMU's names, held together) that type a character with win32k's German layout
/// (`poly_win32k::kbd`: `Y` and `Z` swapped, `/` on Shift+7, `:` on Shift+.).
fn qcodes_de(c: char) -> Option<Vec<String>> {
    let key = |k: &str| Some(vec![k.to_string()]);
    let shift = |k: &str| Some(vec!["shift".to_string(), k.to_string()]);
    match c {
        'z' => key("y"),
        'y' => key("z"),
        'Z' => shift("y"),
        'Y' => shift("z"),
        'a'..='z' | '0'..='9' => key(&c.to_string()),
        'A'..='Z' => shift(&c.to_ascii_lowercase().to_string()),
        ' ' => key("spc"),
        '.' => key("dot"),
        ',' => key("comma"),
        '-' => key("slash"),
        '#' => key("backslash"),
        '+' => key("bracket_right"),
        ':' => shift("dot"),
        ';' => shift("comma"),
        '_' => shift("slash"),
        '/' => shift("7"),
        '?' => shift("minus"),
        '=' => shift("0"),
        '&' => shift("6"),
        '%' => shift("5"),
        '!' => shift("1"),
        '"' => shift("2"),
        '(' => shift("8"),
        ')' => shift("9"),
        '\'' => shift("backslash"),
        '@' => Some(vec!["ctrl".into(), "alt".into(), "q".into()]),
        _ => None,
    }
}

#[cfg(unix)]
struct Driver {
    qmp: crate::screen::Qmp,
    log: PathBuf,
    out: PathBuf,
    name: String,
    query: String,
    shots: u32,
}

#[cfg(unix)]
impl Driver {
    fn events(&mut self, events: &[String]) -> Result {
        self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, events.join(", ")))?;
        std::thread::sleep(Duration::from_millis(120));
        Ok(())
    }
    fn key_ev(q: &str, down: bool) -> String {
        format!(r#"{{"type": "key", "data": {{"down": {down}, "key": {{"type": "qcode", "data": "{q}"}}}}}}"#)
    }
    fn btn(down: bool, b: &str) -> String {
        format!(r#"{{"type": "btn", "data": {{"down": {down}, "button": "{b}"}}}}"#)
    }
    fn chord(&mut self, keys: &[String]) -> Result {
        for k in keys {
            self.events(&[Self::key_ev(k, true)])?;
        }
        for k in keys.iter().rev() {
            self.events(&[Self::key_ev(k, false)])?;
        }
        Ok(())
    }
    fn at(&mut self, x: i32, y: i32) -> Result {
        let (w, h) = (i64::from(crate::desktop::SIZE.0) - 1, i64::from(crate::desktop::SIZE.1) - 1);
        let ax = (i64::from(x) * 0x7fff + w - 1) / w;
        let ay = (i64::from(y) * 0x7fff + h - 1) / h;
        self.events(&[
            format!(r#"{{"type": "abs", "data": {{"axis": "x", "value": {ax}}}}}"#),
            format!(r#"{{"type": "abs", "data": {{"axis": "y", "value": {ay}}}}}"#),
        ])?;
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    }
    fn click(&mut self, x: i32, y: i32, button: &str, times: u32) -> Result {
        self.at(x, y)?;
        for _ in 0..times {
            self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, Self::btn(true, button)))?;
            self.qmp.execute("input-send-event", &format!(r#"{{"events": [{}]}}"#, Self::btn(false, button)))?;
            std::thread::sleep(Duration::from_millis(40));
        }
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }
    fn text(&self) -> String {
        std::fs::read(&self.log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
    }
    fn wait(&self, needle: &str, secs: u64) -> Result<()> {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self.text().contains(needle) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err(format!("no {needle:?} in the log within {secs} s"))
    }
    fn step(&mut self, step: &Step) -> Result {
        match step {
            Step::Launch => {
                let query = self.query.clone();
                for c in query.chars() {
                    let (q, shift) = qcode(c).ok_or(format!("cannot type {c:?}"))?;
                    if shift {
                        self.chord(&["shift".into(), q])?;
                    } else {
                        self.chord(&[q])?;
                    }
                }
                self.wait(&format!("launcher query {query:?}"), 30)?;
                std::thread::sleep(Duration::from_millis(800));
                self.chord(&["ret".into()])?;
                self.wait("poly-compositor: launched", 30)?;
            }
            Step::Sleep(s) => std::thread::sleep(Duration::from_secs(*s)),
            Step::Wait(t) => self.wait(t, 60)?,
            Step::Shot(n) => {
                self.shots += 1;
                let png = self.out.join(format!("{}-{:02}-{n}.png", self.name, self.shots));
                self.qmp.screendump(&png)?;
                eprintln!("==> screenshot {}", png.display());
            }
            Step::Pixel(x, y, want) | Step::Near(x, y, want, _) => {
                let ppm = self.out.join(format!("{}-pixel.ppm", self.name));
                self.qmp.screendump(&ppm)?;
                let image = crate::screen::Image::read_ppm(&ppm)?;
                let (px, py) = (u32::try_from(*x).unwrap_or(u32::MAX), u32::try_from(*y).unwrap_or(u32::MAX));
                if px >= image.width || py >= image.height {
                    return Err(format!("pixel {x},{y} is outside the {}x{} screen", image.width, image.height));
                }
                let got = image.pixel(px, py);
                if let Step::Near(.., t) = step {
                    if got.iter().zip(want).any(|(g, w)| g.abs_diff(*w) > *t) {
                        return Err(format!("pixel {x},{y} is {got:02x?}, wanted {want:02x?} within {t}"));
                    }
                } else if got != *want {
                    return Err(format!("pixel {x},{y} is {got:02x?}, wanted {want:02x?}"));
                }
                eprintln!("==> pixel {x},{y} is {got:02x?}");
            }
            Step::Type(t) => {
                for c in t.chars() {
                    let (q, shift) = qcode(c).ok_or(format!("cannot type {c:?}"))?;
                    if shift {
                        self.chord(&["shift".into(), q])?;
                    } else {
                        self.chord(&[q])?;
                    }
                }
            }
            Step::WType(t) => {
                for c in t.chars() {
                    let keys = qcodes_de(c).ok_or(format!("cannot type {c:?} with the German layout"))?;
                    self.chord(&keys)?;
                }
            }
            Step::Key(k) => self.chord(std::slice::from_ref(k))?,
            Step::Chord(k) => self.chord(k)?,
            Step::Click(x, y) => self.click(*x, *y, "left", 1)?,
            Step::DClick(x, y) => self.click(*x, *y, "left", 2)?,
            Step::RClick(x, y) => self.click(*x, *y, "right", 1)?,
            Step::Drag(x1, y1, x2, y2) => {
                self.at(*x1, *y1)?;
                self.events(&[Self::btn(true, "left")])?;
                std::thread::sleep(Duration::from_millis(400));
                for i in 1..=8 {
                    self.at(x1 + (x2 - x1) * i / 8, y1 + (y2 - y1) * i / 8)?;
                }
                self.events(&[Self::btn(false, "left")])?;
                std::thread::sleep(Duration::from_millis(200));
            }
            Step::Wheel(x, y, n) => {
                self.at(*x, *y)?;
                let b = if *n > 0 { "wheel-down" } else { "wheel-up" };
                for _ in 0..n.unsigned_abs() {
                    self.events(&[Self::btn(true, b)])?;
                    self.events(&[Self::btn(false, b)])?;
                }
            }
            Step::Move(x, y) => self.at(*x, *y)?,
        }
        Ok(())
    }
}

#[cfg(unix)]
fn drive(o: &DriveSpec, socket: &Path, log: &Path) -> Result {
    let qmp = crate::screen::Qmp::connect(socket, Duration::from_secs(30))?;
    let mut d = Driver {
        qmp,
        log: log.to_path_buf(),
        out: o.out.clone(),
        name: o.name.clone(),
        query: o.query.clone(),
        shots: 0,
    };
    d.wait(crate::desktop::READY, 120)?;
    std::thread::sleep(Duration::from_secs(1));
    let started = Instant::now();
    for s in &o.steps {
        if started.elapsed() > Duration::from_secs(o.timeout) {
            return Err("the run's time limit".into());
        }
        if let Err(e) = d.step(s) {
            // What the screen shows when a step fails (a kernel panic, a window that never came).
            let _ = d.step(&Step::Shot("failed".into()));
            return Err(format!("{s:?}: {e}"));
        }
    }
    d.qmp.quit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percent_in_the_arguments_is_not_a_field_code() {
        let f = desktop_file("chrome", "/mnt/root/c.exe", "data:text/html,<b>a%20b</b>");
        assert!(f.contains("\nExec=/mnt/root/c.exe data:text/html,<b>a%%20b</b>\n"), "{f}");
    }

    #[test]
    fn steps_parse() {
        assert_eq!(Step::parse("launch"), Ok(Step::Launch));
        assert_eq!(Step::parse("sleep:5"), Ok(Step::Sleep(5)));
        assert_eq!(Step::parse("click:10|20"), Ok(Step::Click(10, 20)));
        assert_eq!(Step::parse("drag:1|2|30|40"), Ok(Step::Drag(1, 2, 30, 40)));
        assert!(Step::parse("drag:1|2|3").is_err());
        assert_eq!(Step::parse("wheel:300|400|3"), Ok(Step::Wheel(300, 400, 3)));
        assert_eq!(Step::parse("wheel:300|400|-2"), Ok(Step::Wheel(300, 400, -2)));
        assert!(Step::parse("wheel:300|400").is_err());
        assert_eq!(Step::parse("move:5|6"), Ok(Step::Move(5, 6)));
        assert_eq!(Step::parse("wtype:a/b"), Ok(Step::WType("a/b".into())));
        assert_eq!(
            split_steps("launch,type:data:text/html,<p>a,b</p>,key:ret,,sleep:2"),
            ["launch", "type:data:text/html,<p>a,b</p>", "key:ret", "sleep:2"]
        );
        assert_eq!(qcodes_de('z'), Some(vec!["y".to_string()]));
        assert_eq!(qcodes_de(':'), Some(vec!["shift".to_string(), "dot".to_string()]));
        assert_eq!(qcodes_de('/'), Some(vec!["shift".to_string(), "7".to_string()]));
        assert_eq!(Step::parse("pixel:200|400|008000"), Ok(Step::Pixel(200, 400, [0, 128, 0])));
        assert!(Step::parse("pixel:200|400|8000").is_err());
        assert_eq!(Step::parse("near:496|300|0000ff|64"), Ok(Step::Near(496, 300, [0, 0, 255], 64)));
        assert!(Step::parse("near:496|300|0000ff").is_err());
        assert!(Step::parse("pixel:200|400").is_err());
        assert_eq!(Step::parse("chord:ctrl+s"), Ok(Step::Chord(vec!["ctrl".into(), "s".into()])));
        assert!(Step::parse("fly").is_err());
    }

    #[test]
    fn keys_of_the_us_layout() {
        assert_eq!(qcode('a'), Some(("a".into(), false)));
        assert_eq!(qcode('A'), Some(("a".into(), true)));
        assert_eq!(qcode(':'), Some(("semicolon".into(), true)));
        assert_eq!(qcode('~'), None);
    }
}
