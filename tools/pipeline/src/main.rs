//! `pipeline`: the deterministic, model-free parts of the agent pipeline
//! (docs/AGENT_PIPELINE.md). Run it as `cargo xtask pipeline <command>`.
//!
//! ```text
//! build [--cc CC] [--out DIR]                    compile the test corpus in tests/linux/
//! trace <test|path>... [--all] [--tracer ptrace|strace|none] [--out-dir DIR] [--print]
//!                                                trace programs (and app tests) on this (real Linux) machine
//! diff [--target linux|poly] [--tests a,b] [--tracer T] [--syscalls exact|names|off|app]
//!      [--ignore-syscall NAME]... [--timing-envelope F] [--baseline SUMMARY] [--strict]
//!      [--kernel ELF] [--build-kernel] [--timeout SECS] [--bin-dir DIR] [--cc CC] [--tests-dir DIR]...
//!      [--jobs N]                                run the corpus on the reference and a target, compare per test
//!                                                (N tests at a time; 1 by default)
//! selfcheck                                      Linux against Linux must be clean (CI)
//! polycheck                                      Linux against Poly: the tests of tests/linux/poly-pass.txt and the
//!                                                visible spec tests of every green entry pass (CI)
//! polyrun <test>... [--tests-dir DIR]... [--kernel ELF] [--build-kernel] [--print]
//!                                                run tests on Poly only (no reference): output and app-mode events
//! initramfs <binary> --out FILE | --list FILE    write (or list) a test initramfs
//! spec new <entry> --agent ID [--oracle-os TEXT] spec sheet skeleton from specs/TEMPLATE.yaml
//! tests split <entry> <dir> --spec-agent ID [--ratio 0.3] [--replace]
//!                                                visible tests into specs/, held-out ones outside the repo
//! handoff record <entry> --spec-agent A --implementer B [--allow-dirty]
//! handoff bundle <entry> --out DIR               what the implementer receives
//! handoff verify [<entry>] [--received DIR]      prove the hand-offs (and no leaks)
//! next [--os linux] [--layer syscall] [--limit N] [--trace FILE]... [--json]
//! claim <id> --agent ID | release <id>           through the coverage tool
//! role <spec|implementer> <entry> --workdir DIR --agent ID [--preset claude|opencode]
//!      [--model M] [--generated DIR] [--revision REV] [--feedback FILE] [--dry-run]
//! loop [--config FILE] [--once] [--entry ID] [--dry-run] [--quiet]
//!                                                the scheduler: runs the whole pipeline unattended
//! loop status [--config FILE] [--json]           what every entry in the loop is doing
//! loop reset <entry> [--config FILE] [--keep-branches]
//!                                                forget an entry (e.g. after the human queue)
//! ```

mod apps;
mod corpus;
mod cpio;
mod diff;
mod green;
mod handoff;
// The host's shared libraries of a program, found as the dynamic linking
// boot tests find them (one source for both tools).
#[path = "../../xtask/src/libsearch.rs"]
mod libsearch;
mod poly;
mod ptrace;
mod queue;
mod roles;
mod runner;
mod scheduler;
mod sha256;
mod store;
mod syscalls;
mod trace;
mod util;

use diff::{Options, Summary, SyscallMode, TestResult, Verdict};
use runner::Tracer;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;
use store::EntryId;
use trace::{NormOptions, Trace};
use util::{Args, Result};

/// Default output directories, relative to the repository root.
pub const BIN_DIR: &str = "target/pipeline/bin";
pub const TRACES_DIR: &str = "target/pipeline/traces";
pub const RUNS_DIR: &str = "target/pipeline/runs";
pub const ROLES_DIR: &str = "target/pipeline/roles";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Booting Poly under QEMU (TCG) takes longer than running on the host.
const POLY_TIMEOUT: Duration = Duration::from_secs(120);

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf()
}

fn timeout(a: &Args, default: Duration) -> Result<Duration> {
    match a.opt("timeout") {
        Some(s) => s.parse::<u64>().map(Duration::from_secs).map_err(|_| format!("--timeout {s}: seconds")),
        None => Ok(default),
    }
}

/// Commands that observe the real OS. An implementer agent must never run
/// them (CLEAN_ROOM.md): `pipeline role` sets POLY_ROLE for the agent.
fn oracle_guard(command: &str) -> Result {
    if std::env::var("POLY_ROLE").is_ok_and(|r| r == "implementer") {
        return Err(format!(
            "`{command}` observes the real OS or the held-out tests; implementer agents may not run it"
        ));
    }
    Ok(())
}

fn build(repo: &Path, a: &Args) -> Result<u8> {
    a.only(&["cc", "out"])?;
    let out = a.opt("out").map_or_else(|| repo.join(BIN_DIR), PathBuf::from);
    let cc = corpus::compiler(a.opt("cc"));
    let m = corpus::build(repo, &out, &cc)?;
    println!("built {} test programs with {} into {}", m.tests.len(), m.compiler, out.display());
    for s in &m.skipped {
        println!("skipped {}: {}", s.name, s.reason);
    }
    Ok(0)
}

/// The corpus binaries: built now, or prebuilt in `--bin-dir`.
fn corpus_binaries(repo: &Path, a: &Args) -> Result<(PathBuf, corpus::Manifest)> {
    let extra: Vec<PathBuf> = a.all("tests-dir").into_iter().map(PathBuf::from).collect();
    if let Some(dir) = a.opt("bin-dir") {
        if !extra.is_empty() {
            return Err("--tests-dir builds programs; it cannot be used with --bin-dir".into());
        }
        let dir = PathBuf::from(dir);
        let m = corpus::Manifest::load(&dir)?;
        return Ok((dir, m));
    }
    let dir = repo.join(BIN_DIR);
    let m = corpus::build_with(repo, &dir, &corpus::compiler(a.opt("cc")), &extra)?;
    Ok((dir, m))
}

fn trace_cmd(repo: &Path, a: &Args) -> Result<u8> {
    oracle_guard("trace")?;
    a.only(&["all", "tracer", "out-dir", "print", "timeout", "bin-dir", "cc", "tests-dir"])?;
    let tracer = Tracer::parse(a.opt("tracer").unwrap_or("ptrace"))?;
    let out_dir = a.opt("out-dir").map_or_else(|| repo.join(TRACES_DIR), PathBuf::from);
    let mut programs: Vec<(String, PathBuf)> = Vec::new();
    let (names, paths): (Vec<&String>, Vec<&String>) =
        a.positional.iter().partition(|p| !p.contains('/') && !Path::new(p.as_str()).is_file());
    for p in paths {
        let name = Path::new(p).file_stem().map_or("program".into(), |s| s.to_string_lossy().into_owned());
        programs.push((name, util::absolute(Path::new(p))));
    }
    let mut app_tests: Vec<(String, apps::Tree)> = Vec::new();
    if a.has("all") || !names.is_empty() {
        let (dir, m) = corpus_binaries(repo, a)?;
        for b in &m.tests {
            if a.has("all") || names.iter().any(|n| **n == b.name) {
                programs.push((b.name.clone(), dir.join(&b.binary)));
            }
        }
        for app in &m.apps {
            if a.has("all") || names.iter().any(|n| **n == app.name) {
                app_tests.push((app.name.clone(), app_tree(repo, &dir, &m, app)?));
            }
        }
        for n in names {
            if !m.has(n) {
                return Err(format!("no test program or app test {n} (see tests/linux/)"));
            }
        }
    }
    if programs.is_empty() && app_tests.is_empty() {
        return Err("usage: pipeline trace <test|path>... | --all".into());
    }
    let runs = programs
        .into_iter()
        .map(|(n, p)| (n, Some(p), None))
        .chain(app_tests.into_iter().map(|(n, t)| (n, None, Some(t))));
    for (name, path, tree) in runs {
        let t = match (path, tree) {
            (Some(path), _) => runner::run(tracer, &path, &name, timeout(a, DEFAULT_TIMEOUT)?)?,
            (None, Some(tree)) => host_app_run(tracer, &tree, &name, timeout(a, DEFAULT_TIMEOUT)?)?,
            (None, None) => unreachable!(),
        };
        let file = out_dir.join(format!("{name}.json"));
        util::write_json(&file, &t)?;
        if a.has("print") {
            print!("{}", trace::render_raw(&t));
        }
        println!("{name}: {} syscalls, {} -> {}", t.syscalls().count(), t.exit, file.display());
    }
    Ok(0)
}

/// Point `<runs>/latest` at the newest run.
fn link_latest(runs: &Path, run: &Path) {
    let latest = runs.join("latest");
    let _ = std::fs::remove_file(&latest);
    if let Some(name) = run.file_name() {
        let _ = std::os::unix::fs::symlink(name, &latest);
    }
}

fn diff_cmd(repo: &Path, a: &Args) -> Result<u8> {
    oracle_guard("diff")?;
    a.only(&[
        "target",
        "tests",
        "tracer",
        "syscalls",
        "ignore-syscall",
        "timing-envelope",
        "baseline",
        "strict",
        "kernel",
        "build-kernel",
        "timeout",
        "bin-dir",
        "cc",
        "run-dir",
        "tests-dir",
        "jobs",
    ])?;
    let target = a.opt("target").unwrap_or("linux");
    let tracer = Tracer::parse(a.opt("tracer").unwrap_or("ptrace"))?;
    let opts = Options {
        syscalls: SyscallMode::parse(a.opt("syscalls").unwrap_or("exact"))?,
        norm: NormOptions { ignore: a.all("ignore-syscall").into_iter().map(String::from).collect() },
        timing_envelope: a
            .opt("timing-envelope")
            .map(|f| f.parse::<f64>().map_err(|_| format!("--timing-envelope {f}: a factor")))
            .transpose()?,
    };
    let baseline: Option<Summary> = a.opt("baseline").map(|p| util::read_json(Path::new(p))).transpose()?;
    let poly = match target {
        "linux" => None,
        "poly" => {
            let kernel = a.opt("kernel").map_or_else(|| poly::default_kernel(repo), PathBuf::from);
            if a.has("build-kernel") || (a.opt("kernel").is_none() && !kernel.exists()) {
                poly::build_kernel(repo)?;
            }
            Some(poly::Target { kernel, timeout: timeout(a, POLY_TIMEOUT)? })
        }
        t => return Err(format!("unknown target {t} (linux, poly)")),
    };

    let (bin_dir, manifest) = corpus_binaries(repo, a)?;
    let wanted: Vec<&str> = a.opt("tests").map(|t| t.split(',').collect()).unwrap_or_default();
    for w in &wanted {
        if !manifest.has(w) {
            return Err(format!("no test program or app test {w}"));
        }
    }
    let runs = repo.join(RUNS_DIR);
    let run_dir = a.opt("run-dir").map_or_else(|| runs.join(format!("{}-{target}", util::now_stamp())), PathBuf::from);
    let tracer_name = a.opt("tracer").unwrap_or("ptrace");
    let mut summary = Summary::new(util::now_utc(), target, ptrace::host_system(), tracer_name, opts.syscalls);
    let jobs = match a.opt("jobs") {
        Some(j) => {
            j.parse::<usize>().ok().filter(|&j| j > 0).ok_or_else(|| format!("--jobs {j}: a positive number"))?
        }
        None => 1,
    };
    let ref_timeout = timeout(a, DEFAULT_TIMEOUT)?;
    // Once the kernel says it cannot run programs, the rest is not booted.
    let unsupported: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let tests: Vec<&corpus::Built> =
        manifest.tests.iter().filter(|b| wanted.is_empty() || wanted.contains(&b.name.as_str())).collect();

    // One test: the reference run, the target run, the comparison.
    let run_one = |b: &&corpus::Built| -> Result<TestResult> {
        let binary = bin_dir.join(&b.binary);
        let name = b.name.as_str();
        let reference = match runner::run(tracer, &binary, name, ref_timeout) {
            Ok(t) => t,
            Err(e) => return Ok(TestResult::not_run(name, Verdict::Error, format!("reference run: {e}"))),
        };
        let ref_file = format!("reference/{name}.json");
        util::write_json(&run_dir.join(&ref_file), &reference)?;
        let target_run: Result<Trace> = match &poly {
            None => runner::run(tracer, &binary, name, ref_timeout),
            Some(p) => {
                if let Some(reason) = unsupported.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                    return Ok(TestResult::not_run(name, Verdict::Unsupported, reason));
                }
                match p.run(name, &binary, &run_dir.join(target)) {
                    Ok((poly::Outcome::Ran(t), _)) => Ok(t),
                    Ok((poly::Outcome::Unsupported(reason), log)) => {
                        // The kernel cannot run programs: no point booting it for every test.
                        let log = log.strip_prefix(&run_dir).unwrap_or(&log).display().to_string();
                        unsupported
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get_or_insert_with(|| format!("not booted: the kernel could not run {name} ({log})"));
                        return Ok(TestResult::not_run(name, Verdict::Unsupported, format!("{reason} ({log})")));
                    }
                    Err(e) => Err(e),
                }
            }
        };
        let target_trace = match target_run {
            Ok(t) => t,
            Err(e) => return Ok(TestResult::not_run(name, Verdict::Error, format!("target run: {e}"))),
        };
        let target_file = format!("{target}/{name}.json");
        util::write_json(&run_dir.join(&target_file), &target_trace)?;
        let mut test_opts = opts.clone();
        if let Some(mode) = &b.syscalls {
            test_opts.syscalls = opts.syscalls.weaker(SyscallMode::parse(mode)?);
        }
        let mut r = TestResult::compared(name, &reference, &target_trace, &test_opts);
        r.files = vec![ref_file, target_file];
        Ok(r)
    };
    for r in util::parallel_map(&tests, jobs, run_one) {
        summary.tests.push(r?);
    }
    let unsupported: Option<String> = unsupported.into_inner().unwrap_or_else(|e| e.into_inner());

    // App tests: the same tree on both systems, compared in app mode
    // unless --syscalls says otherwise.
    let app_opts = Options {
        syscalls: if a.opt("syscalls").is_some() { opts.syscalls } else { SyscallMode::App },
        ..opts.clone()
    };
    let mut no_jail: Option<String> = None;
    for app in manifest.apps.iter().filter(|b| wanted.is_empty() || wanted.contains(&b.name.as_str())) {
        let name = app.name.as_str();
        let tree = match app_tree(repo, &bin_dir, &manifest, app) {
            Ok(t) => t,
            Err(e) => {
                let verdict = if e.contains("not installed") { Verdict::Unsupported } else { Verdict::Error };
                summary.tests.push(TestResult::not_run(name, verdict, format!("app tree: {e}")));
                continue;
            }
        };
        util::write(&run_dir.join(format!("trees/{name}.txt")), tree.listing())?;
        if let Some(reason) = &no_jail {
            summary.tests.push(TestResult::not_run(name, Verdict::Unsupported, reason.clone()));
            continue;
        }
        let reference = match host_app_run(tracer, &tree, name, timeout(a, DEFAULT_TIMEOUT)?) {
            Ok(t) => t,
            Err(e) => {
                let unsupported = e.contains("not permitted") || e.contains("ptrace tracer only");
                if unsupported {
                    no_jail = Some(format!("reference run: {e}"));
                }
                let verdict = if unsupported { Verdict::Unsupported } else { Verdict::Error };
                summary.tests.push(TestResult::not_run(name, verdict, format!("reference run: {e}")));
                continue;
            }
        };
        let ref_file = format!("reference/{name}.json");
        util::write_json(&run_dir.join(&ref_file), &reference)?;
        let target_run: Result<Trace> = match (&poly, &unsupported) {
            (None, _) => host_app_run(tracer, &tree, name, timeout(a, DEFAULT_TIMEOUT)?),
            (Some(_), Some(reason)) => {
                summary.tests.push(TestResult::not_run(name, Verdict::Unsupported, reason.clone()));
                continue;
            }
            (Some(p), None) => {
                let sha = reference.binary_sha256.clone().unwrap_or_default();
                match p.run_image(name, &tree.cpio()?, &sha, &run_dir.join(target)) {
                    Ok((poly::Outcome::Ran(t), _)) => Ok(t),
                    Ok((poly::Outcome::Unsupported(reason), log)) => {
                        let log = log.strip_prefix(&run_dir).unwrap_or(&log).display().to_string();
                        summary.tests.push(TestResult::not_run(
                            name,
                            Verdict::Unsupported,
                            format!("{reason} ({log})"),
                        ));
                        continue;
                    }
                    Err(e) => Err(e),
                }
            }
        };
        let target_trace = match target_run {
            Ok(t) => t,
            Err(e) => {
                summary.tests.push(TestResult::not_run(name, Verdict::Error, format!("target run: {e}")));
                continue;
            }
        };
        let target_file = format!("{target}/{name}.json");
        util::write_json(&run_dir.join(&target_file), &target_trace)?;
        let mut r = TestResult::compared(name, &reference, &target_trace, &app_opts);
        r.files = vec![ref_file, target_file];
        r.syscall_mode = Some(app_opts.syscalls);
        summary.tests.push(r);
    }
    summary.finish(baseline.as_ref());
    let table = summary.table();
    util::write_json(&run_dir.join("summary.json"), &summary)?;
    util::write(&run_dir.join("report.txt"), &table)?;
    if a.opt("run-dir").is_none() {
        link_latest(&runs, &run_dir);
    }
    print!("{table}");
    println!("run directory: {}", run_dir.display());
    Ok(if summary.failed(baseline.is_some(), a.has("strict")) { 1 } else { 0 })
}

/// The file tree of app test `app` of the corpus build in `bin_dir`.
fn app_tree(repo: &Path, bin_dir: &Path, m: &corpus::Manifest, app: &corpus::App) -> Result<apps::Tree> {
    let launcher = m.launcher.as_ref().ok_or("the corpus build has no app launcher")?;
    let def = apps::AppDef::load(&corpus::Manifest::definition_path(repo, app))?;
    apps::build_tree(&def, &util::read(&bin_dir.join(&launcher.binary))?)
}

/// Run an app test on this machine: its tree written to a fresh directory
/// (removed afterwards), the launcher chrooted into it.
fn host_app_run(tracer: Tracer, tree: &apps::Tree, name: &str, timeout: Duration) -> Result<Trace> {
    let dir = apps::fresh_tree_dir(name);
    let result = tree
        .materialise(&dir)
        .and_then(|()| apps::proc_overlays(&dir))
        .and_then(|overlays| runner::run_app(tracer, &dir, &overlays, name, timeout));
    // The host's /dev and /proc were bound into the tree only in the
    // launcher's own mount namespace, which ended with it: here the tree is
    // plain files.
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(dir.with_extension("filesystems"));
    result
}

/// Linux against Linux over the whole corpus: must be clean. Skipped (with
/// a message, successfully) where there is no C compiler or ptrace is not
/// permitted, so it needs nothing beyond a normal Linux machine.
fn selfcheck(repo: &Path) -> Result<u8> {
    // `cargo xtask ci` runs this, and implementers run CI: skip rather than
    // fail, without running anything on the host.
    if oracle_guard("selfcheck").is_err() {
        println!("pipeline selfcheck: skipped (implementer agents do not run programs on the reference OS)");
        return Ok(0);
    }
    let cc = corpus::compiler(None);
    if corpus::compiler_version(&cc).is_err() {
        println!("pipeline selfcheck: skipped (no C compiler; install gcc or set CC)");
        return Ok(0);
    }
    let base = repo.join("target/pipeline/selfcheck");
    let bin = base.join("bin");
    let m = corpus::build(repo, &bin, &cc)?;
    let hello = bin.join("hello");
    match ptrace::trace(&hello, "hello", DEFAULT_TIMEOUT) {
        Err(e) if e.contains("not permitted") => {
            println!("pipeline selfcheck: skipped ({e})");
            return Ok(0);
        }
        Err(e) => return Err(e),
        Ok(t) => check_hello(&t)?,
    }
    let args: Vec<String> = ["--target", "linux", "--bin-dir"]
        .iter()
        .map(|s| s.to_string())
        .chain([bin.display().to_string(), "--run-dir".into(), base.join("run").display().to_string()])
        .collect();
    let code = diff_cmd(repo, &Args::parse(&args, &["strict", "build-kernel"])?)?;
    if code != 0 {
        return Err("Linux against Linux is not clean: the tracer or the normalisation is nondeterministic".into());
    }
    println!(
        "pipeline selfcheck: {} test programs and {} app tests, Linux against Linux clean",
        m.tests.len(),
        m.apps.len()
    );
    Ok(0)
}

/// The corpus tests that pass on Poly, one name per line (`#` comments):
/// `polycheck` fails if one of them does not.
pub const POLY_PASS_LIST: &str = "tests/linux/poly-pass.txt";

/// The tests [`POLY_PASS_LIST`] names.
fn poly_pass_list(repo: &Path) -> Result<Vec<String>> {
    let text = util::read_string(&repo.join(POLY_PASS_LIST))?;
    Ok(text
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Poly test runs at a time in `polycheck`: each QEMU (TCG, two vCPUs)
/// keeps about two cores busy, and CI runners have four.
const POLYCHECK_MAX_JOBS: usize = 4;

/// Linux against Poly over the whole corpus and the visible spec tests of
/// every green coverage entry ([`green::entries`]): every test in
/// [`POLY_PASS_LIST`] and every such entry test must pass (CI), so a green
/// entry stays green. Skipped, successfully, like `selfcheck`: without a C
/// compiler, where ptrace is not permitted, without QEMU, and for
/// implementer agents (it runs the reference OS).
fn polycheck(repo: &Path) -> Result<u8> {
    if oracle_guard("polycheck").is_err() {
        println!("pipeline polycheck: skipped (implementer agents do not run programs on the reference OS)");
        return Ok(0);
    }
    let cc = corpus::compiler(None);
    if corpus::compiler_version(&cc).is_err() {
        println!("pipeline polycheck: skipped (no C compiler; install gcc or set CC)");
        return Ok(0);
    }
    if Command::new("qemu-system-x86_64").arg("--version").output().is_err() {
        println!("pipeline polycheck: skipped (no qemu-system-x86_64)");
        return Ok(0);
    }
    let base = repo.join("target/pipeline/polycheck");
    let bin = base.join("bin");
    let green = green::entries(repo)?;
    let dirs: Vec<PathBuf> = green.iter().map(|g| repo.join(&g.tests_dir)).collect();
    corpus::build_with(repo, &bin, &cc, &dirs)?;
    if let Err(e) = ptrace::trace(&bin.join("hello"), "hello", DEFAULT_TIMEOUT)
        && e.contains("not permitted")
    {
        println!("pipeline polycheck: skipped ({e})");
        return Ok(0);
    }
    let run_dir = base.join("run");
    let _ = std::fs::remove_dir_all(&run_dir);
    let jobs = util::default_jobs(POLYCHECK_MAX_JOBS).to_string();
    let args: Vec<String> = ["--target", "poly", "--build-kernel", "--jobs", &jobs, "--bin-dir"]
        .iter()
        .map(|s| s.to_string())
        .chain([bin.display().to_string(), "--run-dir".into(), run_dir.display().to_string()])
        .collect();
    // Failures of tests not on the list are expected: they are reported in
    // the table, and the list and the green entries decide.
    diff_cmd(repo, &Args::parse(&args, &["strict", "build-kernel"])?)?;
    let mut summary: Summary = util::read_json(&run_dir.join("summary.json"))?;
    // The spec tests were recorded as root (mount, set*id, capabilities,
    // permission checks). Run by anyone else (GitHub's CI runs as a normal
    // user) the reference fails them or traces other ids (getuid() = 1001),
    // so the comparison says nothing about Poly; a green entry's test then
    // only has to pass its own checks on Poly (they print ok / not ok and
    // exit 0 when all pass).
    // The full comparison runs where the reference is root.
    let mut self_checked = Vec::new();
    if !running_as_root() {
        let green_tests: Vec<&str> = green.iter().flat_map(|g| g.tests.iter().map(String::as_str)).collect();
        for t in &mut summary.tests {
            if t.verdict == Verdict::Fail
                && green_tests.contains(&t.test.as_str())
                && t.target_exit.as_deref() == Some("exit 0")
            {
                t.verdict = Verdict::Pass;
                self_checked.push(t.test.clone());
            }
        }
    }
    if !self_checked.is_empty() {
        println!(
            "pipeline polycheck: not root, so these green-entry tests only passed their own checks on Poly (they \
             were recorded as root): {}",
            self_checked.join(", ")
        );
    }
    let expected = poly_pass_list(repo)?;
    // App tests this host cannot run as the reference (no chroot allowed,
    // a program not installed) say nothing about Poly: skipped, like the
    // whole check where ptrace is not permitted.
    let host_cannot = |name: &str| {
        summary.tests.iter().any(|t| {
            t.test == name
                && t.verdict == Verdict::Unsupported
                && t.reason.as_deref().is_some_and(|r| r.starts_with("reference run:") || r.starts_with("app tree:"))
        })
    };
    let skipped: Vec<&str> = expected.iter().map(String::as_str).filter(|t| host_cannot(t)).collect();
    if !skipped.is_empty() {
        println!(
            "pipeline polycheck: skipped on this host (its reference run is not possible): {}",
            skipped.join(", ")
        );
    }
    let checked: Vec<String> = expected.iter().filter(|t| !host_cannot(t)).cloned().collect();
    let c = green::check(&summary, &checked, &green);
    if !c.newly_passing.is_empty() {
        println!("pipeline polycheck: also passing now (add them to {POLY_PASS_LIST}): {}", c.newly_passing.join(", "));
    }
    let report = run_dir.join("report.txt");
    let problems = c.problems(POLY_PASS_LIST);
    if !problems.is_empty() {
        return Err(format!("on Poly, {} (report: {})", problems.join("; "), report.display()));
    }
    let entry_tests: usize = green.iter().map(|g| g.tests.len()).sum();
    println!(
        "pipeline polycheck: the {} tests of {POLY_PASS_LIST}{} and the {entry_tests} visible tests of {} green \
         entries pass on Poly",
        checked.len(),
        if skipped.is_empty() { String::new() } else { format!(" ({} skipped)", skipped.len()) },
        green.len()
    );
    Ok(0)
}

/// Whether this process runs as root (`/proc/self` belongs to its user).
fn running_as_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// The freestanding hello world makes exactly two syscalls.
fn check_hello(t: &Trace) -> Result {
    let lines: Vec<String> = trace::normalize(t, &NormOptions::default()).iter().map(ToString::to_string).collect();
    let want = ["write(1, \"hello, world\\n\", 13) = 13", "exit_group(0) = ?"];
    if lines != want || t.stdout != "hello, world\n" || t.exit != trace::Exit::Code(0) {
        return Err(format!("unexpected hello trace: {lines:?}, stdout {:?}, {}", t.stdout, t.exit));
    }
    Ok(())
}

/// Run corpus programs and app tests on Poly only, without a reference
/// run (so implementer agents may use it): per test the exit status,
/// stdout, stderr and, with `--print`, the app-mode event sequence
/// (`diff::app_events`), kept in the run directory as `<test>.txt`.
fn polyrun(repo: &Path, a: &Args) -> Result<u8> {
    a.only(&["kernel", "build-kernel", "timeout", "cc", "tests-dir", "print", "run-dir"])?;
    let kernel = a.opt("kernel").map_or_else(|| poly::default_kernel(repo), PathBuf::from);
    if a.has("build-kernel") || (a.opt("kernel").is_none() && !kernel.exists()) {
        poly::build_kernel(repo)?;
    }
    let target = poly::Target { kernel, timeout: timeout(a, POLY_TIMEOUT)? };
    let (bin_dir, manifest) = corpus_binaries(repo, a)?;
    let runs = repo.join(RUNS_DIR);
    let run_dir = a.opt("run-dir").map_or_else(|| runs.join(format!("{}-polyrun", util::now_stamp())), PathBuf::from);
    if a.positional.is_empty() {
        return Err("usage: pipeline polyrun <test>... [--tests-dir DIR]...".into());
    }
    for name in &a.positional {
        let run = if let Some(app) = manifest.apps.iter().find(|t| &t.name == name) {
            let tree = app_tree(repo, &bin_dir, &manifest, app)?;
            target.run_image(name, &tree.cpio()?, "", &run_dir)
        } else if let Some(b) = manifest.tests.iter().find(|t| &t.name == name) {
            target.run(name, &bin_dir.join(&b.binary), &run_dir)
        } else {
            return Err(format!("no test program or app test {name}"));
        };
        let t = match run? {
            (poly::Outcome::Ran(t), _) => t,
            (poly::Outcome::Unsupported(r), _) => {
                println!("== {name}: unsupported: {r}");
                continue;
            }
        };
        let mut text = format!("== {name}: {}\n-- stdout:\n{}-- stderr:\n{}-- events:\n", t.exit, t.stdout, t.stderr);
        for e in diff::app_events(&t, &NormOptions::default()) {
            text.push_str(&format!("{e}\n"));
        }
        util::write(&run_dir.join(format!("{name}.txt")), &text)?;
        if a.has("print") {
            print!("{text}");
        } else {
            print!("{}", text.split("-- events:").next().unwrap_or_default());
        }
    }
    println!("run directory: {}", run_dir.display());
    Ok(0)
}

fn initramfs(a: &Args) -> Result<u8> {
    a.only(&["out", "list"])?;
    if let Some(file) = a.opt("list") {
        for e in cpio::read(&util::read(Path::new(file))?)? {
            println!("{:06o} {:>8} {}", e.mode, e.data.len(), e.name);
        }
        return Ok(0);
    }
    let [binary] = a.positional.as_slice() else { return Err("usage: pipeline initramfs <binary> --out FILE".into()) };
    let out = a.require("out")?;
    util::write(Path::new(out), poly::initramfs(&util::read(Path::new(binary))?)?)?;
    println!("{out}: {binary} as {}; boot with -initrd {out} -append \"{}\"", poly::INIT_PATH, poly::KERNEL_ARGS);
    Ok(0)
}

fn spec(repo: &Path, a: &Args) -> Result<u8> {
    oracle_guard("spec")?;
    a.only(&["agent", "oracle-os"])?;
    let [sub, id] = a.positional.as_slice() else {
        return Err("usage: pipeline spec new <entry> --agent ID [--oracle-os TEXT]".into());
    };
    if sub != "new" {
        return Err(format!("unknown spec command {sub}"));
    }
    let entry = EntryId::parse(id)?;
    let agent = a.require("agent")?;
    let path = repo.join(entry.spec_path());
    if path.exists() {
        return Err(format!("{} exists already", entry.spec_path().display()));
    }
    let oracle = match (a.opt("oracle-os"), entry.os.as_str()) {
        (Some(o), _) => o.to_string(),
        (None, "linux") => ptrace::host_system().replacen("linux", "Linux", 1),
        (None, _) => "TODO".into(),
    };
    let template = util::read_string(&repo.join("specs/TEMPLATE.yaml"))?;
    let date = util::now_utc()[..10].to_string();
    util::write(&path, store::spec_skeleton(&template, &entry, &oracle, agent, &date))?;
    util::create_dir(&repo.join(entry.tests_dir()))?;
    println!("created {}", entry.spec_path().display());
    Ok(0)
}

fn tests_cmd(repo: &Path, a: &Args) -> Result<u8> {
    oracle_guard("tests")?;
    a.only(&["spec-agent", "ratio", "replace"])?;
    let [sub, id, generated] = a.positional.as_slice() else {
        return Err(
            "usage: pipeline tests split <entry> <generated-dir> --spec-agent ID [--ratio 0.3] [--replace]".into()
        );
    };
    if sub != "split" {
        return Err(format!("unknown tests command {sub}"));
    }
    let entry = EntryId::parse(id)?;
    let heldout = store::heldout_root(repo)?;
    let ratio = a.opt("ratio").unwrap_or("0.3").parse::<f64>().map_err(|e| format!("--ratio: {e}"))?;
    let revision = util::git_text(repo, &["rev-parse", "HEAD"]).unwrap_or_else(|_| "unknown".into());
    let req = store::SplitRequest {
        entry: &entry,
        generated: Path::new(generated),
        ratio,
        spec_agent: a.require("spec-agent")?,
        revision: &revision,
        replace: a.has("replace"),
    };
    let s = store::split_tests(repo, &heldout, &req)?;
    println!(
        "{}: {} visible in {}, {} held out in {} (manifest {})",
        entry.id(),
        s.visible.len(),
        entry.tests_dir().display(),
        s.held_out.len(),
        entry.heldout_dir(&heldout).display(),
        s.manifest_path.display()
    );
    Ok(0)
}

/// The held-out store if this machine has one (implementers do not).
fn heldout_if_present(repo: &Path) -> Result<Option<PathBuf>> {
    let h = store::heldout_root(repo)?;
    Ok(h.is_dir().then_some(h))
}

fn handoff_cmd(repo: &Path, a: &Args) -> Result<u8> {
    a.only(&["spec-agent", "implementer", "allow-dirty", "out", "received"])?;
    match a.positional.first().map(String::as_str) {
        Some("record") => {
            let [_, id] = a.positional.as_slice() else {
                return Err("usage: pipeline handoff record <entry> ...".into());
            };
            let entry = EntryId::parse(id)?;
            let heldout = heldout_if_present(repo)?;
            if heldout.is_none() {
                eprintln!("warning: no held-out store here; cannot check the hand-off for held-out content");
            }
            let req = handoff::RecordRequest {
                entry: &entry,
                spec_agent: a.require("spec-agent")?,
                implementer: a.require("implementer")?,
                allow_dirty: a.has("allow-dirty"),
                heldout: heldout.as_deref(),
            };
            let r = handoff::record(repo, &req)?;
            println!("logged hand-off of {} at {} ({} files) in {}", r.entry, r.revision, r.files.len(), handoff::LOG);
            Ok(0)
        }
        Some("bundle") => {
            let [_, id] = a.positional.as_slice() else {
                return Err("usage: pipeline handoff bundle <entry> --out DIR".into());
            };
            let out = PathBuf::from(a.require("out")?);
            let r = handoff::bundle(repo, id, &out, heldout_if_present(repo)?.as_deref())?;
            println!("{} files of the {} hand-off at {} in {}", r.files.len(), r.entry, r.revision, out.display());
            Ok(0)
        }
        Some("verify") => {
            let entry = a.positional.get(1).map(String::as_str);
            let heldout = heldout_if_present(repo)?;
            let received = a.opt("received").map(PathBuf::from);
            let req = handoff::VerifyRequest { entry, heldout: heldout.as_deref(), received: received.as_deref() };
            let problems = handoff::verify(repo, &req)?;
            for p in &problems {
                println!("problem: {p}");
            }
            if heldout.is_none() {
                println!("note: no held-out store here; held-out leak checks were skipped");
            }
            if problems.is_empty() {
                println!("hand-offs verified");
                Ok(0)
            } else {
                Ok(1)
            }
        }
        _ => Err("usage: pipeline handoff <record|bundle|verify> ...".into()),
    }
}

fn next_cmd(repo: &Path, a: &Args) -> Result<u8> {
    a.only(&["os", "layer", "limit", "trace", "json"])?;
    let files: Vec<PathBuf> = match a.all("trace") {
        t if t.is_empty() => queue::default_traces(repo),
        t => t.into_iter().map(PathBuf::from).collect(),
    };
    let traces: Vec<Trace> = files.iter().map(|f| util::read_json(f)).collect::<Result<_>>()?;
    let limit = a.opt("limit").unwrap_or("10").parse::<usize>().map_err(|e| format!("--limit: {e}"))?;
    let items = queue::next(repo, a.opt("os").unwrap_or("linux"), a.opt("layer").unwrap_or("syscall"), &traces)?;
    let items = &items[..limit.min(items.len())];
    if a.has("json") {
        println!("{}", serde_json::to_string_pretty(items).map_err(|e| e.to_string())?);
        return Ok(0);
    }
    if traces.is_empty() {
        eprintln!("note: no startup traces; run `cargo xtask pipeline trace hello libc-hello` for first-use order");
    }
    for (i, it) in items.iter().enumerate() {
        let first = it.first_use.map_or("-".into(), |f| f.to_string());
        println!("{:>3}  {:<45} weight {:<6} first use {:<4} {}", i + 1, it.id, it.weight, first, it.status);
    }
    Ok(0)
}

fn coverage_tool(repo: &Path, args: &[String]) -> Result<u8> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(repo)
        .args(["run", "--quiet", "--package", "poly-coverage", "--"])
        .args(args)
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    Ok(if status.success() { 0 } else { 1 })
}

fn role_cmd(repo: &Path, a: &Args) -> Result<u8> {
    a.only(&["workdir", "agent", "preset", "model", "generated", "revision", "feedback", "dry-run"])?;
    let [role, id] = a.positional.as_slice() else {
        return Err("usage: pipeline role <spec|implementer> <entry> --workdir DIR --agent ID ...".into());
    };
    let role = roles::Role::parse(role)?;
    let entry = EntryId::parse(id)?;
    let command = match (std::env::var(role.env_var()), a.opt("preset")) {
        (_, Some(p)) => roles::preset(p, role, a.opt("model"))?,
        (Ok(c), None) if !c.is_empty() => c,
        _ => return Err(format!("set {} or pass --preset claude|opencode", role.env_var())),
    };
    let revision = match (role, a.opt("revision")) {
        (_, Some(r)) => Some(r.to_string()),
        (roles::Role::Implementer, None) => Some(handoff::latest(repo, &entry.id())?.revision),
        (roles::Role::Spec, None) => None,
    };
    let feedback = a.opt("feedback").map(|f| util::read_string(Path::new(f))).transpose()?;
    let req = roles::Request {
        role,
        entry: &entry,
        agent: a.require("agent")?,
        workdir: Path::new(a.require("workdir")?),
        command: &command,
        generated: a.opt("generated").map(Path::new),
        revision: revision.as_deref(),
        feedback,
        dry_run: a.has("dry-run"),
    };
    let out = roles::run(repo, &repo.join(ROLES_DIR), &req)?;
    if out.limit_reached {
        eprintln!("usage limit reached: pause this role and run it again later (log: {})", out.log.display());
        return Ok(roles::LIMIT_EXIT);
    }
    if !out.success {
        eprintln!("the {} agent failed (log: {})", role.name(), out.log.display());
        return Ok(1);
    }
    Ok(0)
}

fn run(args: &[String]) -> Result<u8> {
    let repo = repo_root();
    let (cmd, rest) = args.split_first().ok_or("usage: pipeline <command> (see tools/pipeline/src/main.rs)")?;
    if cmd == "loop" {
        // Its own options and switches.
        return scheduler::command(&repo, rest);
    }
    let switches = ["all", "print", "strict", "build-kernel", "replace", "allow-dirty", "json", "dry-run"];
    let a = Args::parse(rest, &switches)?;
    match cmd.as_str() {
        "build" => build(&repo, &a),
        "trace" => trace_cmd(&repo, &a),
        "diff" => diff_cmd(&repo, &a),
        "selfcheck" => selfcheck(&repo),
        "polycheck" => polycheck(&repo),
        "initramfs" => initramfs(&a),
        "polyrun" => polyrun(&repo, &a),
        "spec" => spec(&repo, &a),
        "tests" => tests_cmd(&repo, &a),
        "handoff" => handoff_cmd(&repo, &a),
        "next" => next_cmd(&repo, &a),
        "claim" | "release" => coverage_tool(&repo, args),
        "role" => role_cmd(&repo, &a),
        _ => Err(format!(
            "unknown command {cmd}; commands: build trace diff selfcheck polycheck polyrun initramfs spec tests handoff next \
             claim release role loop"
        )),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("pipeline: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real tracer on the real hello world, where a C compiler exists.
    #[test]
    fn traces_hello_world() {
        let cc = corpus::compiler(None);
        if corpus::compiler_version(&cc).is_err() {
            eprintln!("skipped: no C compiler");
            return;
        }
        let dir = std::env::temp_dir().join(format!("poly-pipeline-hello-{}", std::process::id()));
        let m = corpus::build(&repo_root(), &dir, &cc).unwrap();
        match ptrace::trace(&dir.join("hello"), "hello", DEFAULT_TIMEOUT) {
            Err(e) if e.contains("not permitted") => eprintln!("skipped: {e}"),
            r => check_hello(&r.unwrap()).unwrap(),
        }
        // A program killed by a signal, and one that fails its checks never
        // happens in the corpus: everything passes on Linux.
        let segv = ptrace::trace(&dir.join("segv"), "segv", DEFAULT_TIMEOUT);
        if let Ok(t) = segv {
            assert_eq!(t.exit, trace::Exit::Signal(11));
            assert!(t.events.iter().any(|e| matches!(e, trace::Event::Signal { signal: 11, .. })));
        }
        let untraced = runner::run(Tracer::None, &dir.join("exit42"), "exit42", DEFAULT_TIMEOUT).unwrap();
        assert_eq!(untraced.exit, trace::Exit::Code(42));
        assert!(m.tests.len() >= 11);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A child that leaves the test's process group (setsid) is still
    /// traced to its end, and one that hangs there is killed on timeout.
    #[test]
    fn traces_children_that_leave_the_process_group() {
        let cc = corpus::compiler(None);
        if corpus::compiler_version(&cc).is_err() {
            eprintln!("skipped: no C compiler");
            return;
        }
        let base = std::env::temp_dir().join(format!("poly-pipeline-setsid-{}", std::process::id()));
        let src = base.join("src");
        util::create_dir(&src).unwrap();
        // fork (57); in the child setsid (112), getppid (110), exit_group 3;
        // the parent waits with wait4 (61) and exits 0. With argument
        // "hang" the child spins after setsid instead.
        let program = r#"#include "poly_test.h"
int test_main(void) {
    i64 pid = syscall6(57, 0, 0, 0, 0, 0, 0);
    if (pid == 0) {
        syscall6(112, 0, 0, 0, 0, 0, 0);
        if (poly_initial_sp[0] > 1) for (;;) {}
        syscall6(110, 0, 0, 0, 0, 0, 0);
        exit_group(3);
    }
    int status = 0;
    syscall6(61, pid, (i64)&status, 0, 0, 0, 0);
    return ((status >> 8) & 0xff) == 3 ? 0 : 1;
}
"#;
        util::write(&src.join("leaves_group.c"), program).unwrap();
        let bin = base.join("bin");
        corpus::build_with(&repo_root(), &bin, &cc, &[src]).unwrap();
        let t = match ptrace::trace(&bin.join("leaves_group"), "leaves_group", DEFAULT_TIMEOUT) {
            Err(e) if e.contains("not permitted") => return eprintln!("skipped: {e}"),
            r => r.unwrap(),
        };
        assert_eq!(t.exit, trace::Exit::Code(0));
        let names: Vec<(u32, &str)> = t
            .events
            .iter()
            .filter_map(|e| match e {
                trace::Event::Syscall(c) => Some((c.pid, c.name.as_str())),
                _ => None,
            })
            .collect();
        assert!(names.contains(&(1, "setsid")) && names.contains(&(1, "getppid")), "{names:?}");
        assert!(names.contains(&(1, "exit_group")) && names.contains(&(0, "wait4")), "{names:?}");

        // The hanging variant: argv[1] given through the trace name is not
        // possible, so run the same binary through a wrapper argument.
        let hang = base.join("hang.sh");
        util::write(&hang, format!("#!/bin/sh\nexec {} hang\n", bin.join("leaves_group").display())).unwrap();
        std::fs::set_permissions(&hang, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let start = std::time::Instant::now();
        let t = ptrace::trace(&hang, "hang", Duration::from_secs(2)).unwrap();
        assert_eq!(t.exit, trace::Exit::Timeout);
        assert!(start.elapsed() < Duration::from_secs(30), "the watchdog reached the session child");
        let _ = std::fs::remove_dir_all(&base);
    }
}
