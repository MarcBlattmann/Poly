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
//!     `type:TEXT`, `key:QCODE`, `chord:ctrl+s`, `click:X,Y`, `dclick:X,Y`, `rclick:X,Y`.
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

/// `polystub.dll` for the directory: a stub for every import of its files that Poly cannot bind.
fn polystub(dir: &Path) -> Result<Vec<u8>> {
    let mods = modules(dir)?;
    let mut ours = crate::winimports::load_ours()?;
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
    let image = poly_stubs::build(&wants);
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
        Some(s) => s.split(',').filter(|p| !p.is_empty()).map(Step::parse).collect::<Result<Vec<_>>>()?,
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
    let stubs = polystub(&o.dir)?;
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
    tree.file(&format!("{}/{}", crate::chromewin::SYSTEM32_ON_DISK, poly_stubs::STUB_DLL), 0o755, &stubs);
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
        for l in text.lines().filter(|l| l.contains("LAUNCH-EXIT")) {
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

/// A desktop run: the compositor is init, a desktop file starts the program, QMP drives it.
#[cfg(unix)]
fn run_desktop(
    o: &Opts,
    kernel: &Path,
    image: &Path,
    log: &Path,
    res: &crate::resources::Resources,
    more: &str,
) -> Result<String> {
    let dos = format!("/mnt/root/{APPS_ON_DISK}/{}/{}", o.name, o.exe);
    let file = format!(
        "[Desktop Entry]\nType=Application\nName=Winapp {n}\nComment=Windows program under test\nExec={dos} {a}\nKeywords=winapp;{n};\n",
        n = o.name,
        a = o.args
    );
    let extra = vec![(format!("usr/share/applications/winapp-{}.desktop", o.name), 0o644, file.into_bytes())];
    let initrd = crate::desktop::image_with("poly-win-app-initramfs.cpio", false, extra)?;
    let socket = std::env::temp_dir().join(format!("poly-win-app-qmp-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    let append = crate::desktop::append(&format!("poly.mount=virtio0:/mnt/root console=ttyS0 poly.net=qemu{more}"));
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
    cmd.arg("-initrd").arg(&initrd).args(crate::desktop::devices());
    cmd.args(["-display", "none", "-qmp", &format!("unix:{},server=on,wait=off", socket.display())])
        .stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
    let result = drive(o, &socket, log);
    let _ = std::fs::remove_file(&socket);
    let _ = child.kill();
    let _ = child.wait();
    if let Err(e) = result {
        eprintln!("==> the steps stopped: {e}");
    }
    Ok(std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
}

#[cfg(not(unix))]
fn run_desktop(_: &Opts, _: &Path, _: &Path, _: &Path, _: &crate::resources::Resources, _: &str) -> Result<String> {
    Err("--mode desktop needs a Unix host (QMP over a Unix socket); run it with polyd".into())
}

/// One step of a desktop run.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Step {
    Launch,
    Sleep(u64),
    Wait(String),
    Shot(String),
    Type(String),
    Key(String),
    Chord(Vec<String>),
    Click(i32, i32),
    DClick(i32, i32),
    RClick(i32, i32),
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
            "type" => Step::Type(arg.into()),
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
        _ => None,
    }
}

#[cfg(unix)]
struct Driver {
    qmp: crate::screen::Qmp,
    log: PathBuf,
    out: PathBuf,
    name: String,
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
                let name = self.name.clone();
                for c in format!("winapp {name}").chars() {
                    let (q, shift) = qcode(c).ok_or(format!("cannot type {c:?}"))?;
                    if shift {
                        self.chord(&["shift".into(), q])?;
                    } else {
                        self.chord(&[q])?;
                    }
                }
                self.wait(&format!("launcher query \"winapp {name}\""), 30)?;
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
            Step::Key(k) => self.chord(std::slice::from_ref(k))?,
            Step::Chord(k) => self.chord(k)?,
            Step::Click(x, y) => self.click(*x, *y, "left", 1)?,
            Step::DClick(x, y) => self.click(*x, *y, "left", 2)?,
            Step::RClick(x, y) => self.click(*x, *y, "right", 1)?,
        }
        Ok(())
    }
}

#[cfg(unix)]
fn drive(o: &Opts, socket: &Path, log: &Path) -> Result {
    let qmp = crate::screen::Qmp::connect(socket, Duration::from_secs(30))?;
    let mut d = Driver { qmp, log: log.to_path_buf(), out: o.out.clone(), name: o.name.clone(), shots: 0 };
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
    fn steps_parse() {
        assert_eq!(Step::parse("launch"), Ok(Step::Launch));
        assert_eq!(Step::parse("sleep:5"), Ok(Step::Sleep(5)));
        assert_eq!(Step::parse("click:10|20"), Ok(Step::Click(10, 20)));
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
