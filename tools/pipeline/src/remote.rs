//! The Windows oracle machine over SSH (ADR 0097): `win remote` builds the
//! Windows tests, bundles them, runs them on a Windows machine reachable
//! as `ssh <host>` with `poly-oracle.ps1` (ADR 0075), fetches the result
//! file back and checks it, and optionally records it as the reference and
//! compares Poly with it.
//!
//! ```text
//! win remote prepare --host H        build + bundle into a stage directory, with transfer.sh
//!   │ sh <stage>/transfer.sh         on a machine with SSH access (the host, not the container):
//!   │                                scp bundle + runner, ssh: run it, scp results.json back
//!   ▼
//! win remote finish                  check the results; --record, --diff
//! win remote --host H                all three where `ssh` works (no container)
//! ```
//!
//! The build runs where the Rust toolchain is (on the owner's PC: the
//! container), SSH where the keys are (the host). The stage directory is
//! inside the repository (`.oracle-remote/`, ignored by git, outside
//! `target/`, which the container keeps in a volume) so both sides see it.
//! Everything the SSH side does is in `transfer.sh`, written here: the
//! PowerShell commands go to the machine encoded (`-EncodedCommand`), so
//! any remote default shell, PowerShell or cmd, passes them on unchanged.

use crate::poly;
use crate::util::{self, Args, Result};
use crate::windows::{self, Manifest, RunFile};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PLAN_FORMAT: &str = "poly-winremote/1";
/// Stage directories, relative to the repository root.
pub const STAGE_ROOT: &str = ".oracle-remote";
/// Where runs go on the Windows machine, relative to the user's home.
pub const DEFAULT_REMOTE_DIR: &str = "poly-oracle-work/remote";
/// The runner, copied with every bundle so both always match.
pub const RUNNER: &str = "tools/pipeline/windows/poly-oracle.ps1";
/// Recorded references of the corpus (ADR 0075), one file per test.
pub const ORACLE_DIR: &str = "tests/windows/oracle";

/// What a stage holds: `plan.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub format: String,
    pub host: String,
    /// The run's directory on the machine, relative to the user's home.
    pub remote_dir: String,
    pub bundle: String,
    pub bundle_id: String,
    /// Extra arguments for `poly-oracle.ps1` (`-Runs 3`, `-TestTimeouts a=60`).
    #[serde(default)]
    pub runner_args: Vec<String>,
    pub tests: Vec<String>,
}

/// An SSH destination: `host`, `user@host`, an alias from the SSH config.
pub fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && !h.starts_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
}

/// A remote directory: relative to the home (or `C:/...`), `/` separated,
/// no `..`, nothing a shell or PowerShell would read specially.
pub fn valid_remote_dir(d: &str) -> bool {
    !d.is_empty()
        && !d.starts_with(['-', '/'])
        && d.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':'))
        && d.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

/// A runner argument (`-Runs`, `3`, `a=60,b=10`): plain words only, since
/// it is pasted into a PowerShell command line.
pub fn valid_runner_arg(a: &str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=' | ',' | '.' | ':'))
}

/// `powershell.exe -EncodedCommand` text: base64 of UTF-16LE.
pub fn encode_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    windows::base64(&bytes)
}

/// The PowerShell commands run on the machine: make the run directory,
/// run the runner there, remove the run directory. Every one starts in the
/// user's home, where `scp`'s relative paths land too. A PowerShell whose
/// output is not a console sends progress, `Write-Host` and errors as
/// CLIXML; so progress is off, the host messages (stream 6) go to standard
/// output and an error is printed as one line of text with exit code 1.
pub fn remote_scripts(plan: &Plan) -> [String; 3] {
    let wrap = |body: String| {
        format!(
            "$ProgressPreference = 'SilentlyContinue'; $ErrorActionPreference = 'Stop'; \
             try {{ Set-Location -LiteralPath $env:USERPROFILE; {body} }} \
             catch {{ [Console]::Error.WriteLine([string]$_); exit 1 }}"
        )
    };
    let d = &plan.remote_dir;
    let mkdir = wrap(format!("New-Item -ItemType Directory -Force -Path '{d}' | Out-Null"));
    let extra: String = plan.runner_args.iter().map(|a| format!(" {a}")).collect();
    let run = wrap(format!(
        "& '{d}/poly-oracle.ps1' -Bundle '{d}/{b}' -Out '{d}/results.json' -Work '{d}/work'{extra} 6>&1",
        b = plan.bundle
    ));
    let clean = wrap(format!("Remove-Item -LiteralPath '{d}' -Recurse -Force"));
    [mkdir, run, clean]
}

/// `transfer.sh`: the SSH side of one run, for a POSIX shell (Git Bash on
/// Windows) with `ssh` and `scp` that reach the host without a password.
pub fn transfer_script(plan: &Plan) -> String {
    let [mkdir, run, clean] = remote_scripts(plan);
    let (h, d) = (&plan.host, &plan.remote_dir);
    let mut s = format!(
        "#!/bin/sh\n\
         # Generated by `cargo xtask pipeline win remote prepare` (ADR 0097); do not edit.\n\
         # Runs bundle {id} ({n} tests) on the Windows oracle {h} and fetches the results.\n\
         # Run it where `ssh {h}` works (the host, not the build container), then\n\
         # `cargo xtask pipeline win remote finish --stage <this directory>`.\n\
         # The remote PowerShell commands, sent encoded:\n",
        id = plan.bundle_id,
        n = plan.tests.len(),
    );
    for (what, script) in [("mkdir", &mkdir), ("run", &run), ("clean", &clean)] {
        s.push_str(&format!("#   {what}: {script}\n"));
    }
    s.push_str(&format!(
        "set -eu\n\
         cd \"$(dirname \"$0\")\"\n\
         remote() {{ ssh -o BatchMode=yes {h} powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand \"$1\"; }}\n\
         rm -f results.json\n\
         remote {mkdir}\n\
         scp -q -o BatchMode=yes {b} poly-oracle.ps1 {h}:{d}/\n\
         status=0\n\
         remote {run} || status=$?\n\
         fetched=0\n\
         scp -q -o BatchMode=yes {h}:{d}/results.json results.json && fetched=1\n\
         remote {clean} || echo \"note: could not remove {d} on {h}\" >&2\n\
         if [ \"$fetched\" != 1 ]; then echo \"no results from {h} (runner exit $status)\" >&2; exit 1; fi\n\
         echo \"results: $(pwd)/results.json (runner exit $status)\"\n\
         exit \"$status\"\n",
        b = plan.bundle,
        mkdir = encode_command(&mkdir),
        run = encode_command(&run),
        clean = encode_command(&clean),
    ));
    s
}

/// Inside a container (no SSH keys there, ADR 0097).
fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

fn ssh_usable() -> bool {
    !in_container() && Command::new("ssh").arg("-V").output().is_ok_and(|o| o.status.success())
}

fn stage_dir(repo: &Path, a: &Args) -> Result<PathBuf> {
    match a.opt("stage") {
        Some(s) => Ok(util::absolute(Path::new(s))),
        None => Ok(repo.join(STAGE_ROOT).join(a.require("host")?)),
    }
}

/// Build the tests, bundle them and write the stage.
fn prepare(repo: &Path, a: &Args, stage: &Path) -> Result<Plan> {
    let host = a.require("host")?;
    if !valid_host(host) {
        return Err(format!("--host {host}: an SSH host name or alias"));
    }
    let base = a.opt("remote-dir").unwrap_or(DEFAULT_REMOTE_DIR).trim_end_matches('/').to_string();
    if !valid_remote_dir(&base) {
        return Err(format!("--remote-dir {base}: letters, digits, . _ - : / and no .."));
    }
    let runner_args: Vec<String> = a.all("runner-arg").into_iter().map(String::from).collect();
    if let Some(bad) = runner_args.iter().find(|r| !valid_runner_arg(r)) {
        return Err(format!("--runner-arg {bad}: letters, digits and - _ = , . : only"));
    }
    let extra: Vec<PathBuf> = a.all("tests-dir").into_iter().map(PathBuf::from).collect();
    let bin = repo.join(windows::WIN_DIR).join("bin");
    let m = windows::build(repo, &bin, &extra, &crate::test_list(a))?;
    let text = windows::bundle(&m, &bin)?;
    let id = windows::bundle_id(&text);
    if stage.exists() {
        std::fs::remove_dir_all(stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    }
    let bundle = format!("bundle-{id}.txt");
    util::write(&stage.join(&bundle), &text)?;
    util::write_json(&stage.join("manifest.json"), &m)?;
    util::write(&stage.join("poly-oracle.ps1"), util::read(&repo.join(RUNNER))?)?;
    let plan = Plan {
        format: PLAN_FORMAT.into(),
        host: host.into(),
        remote_dir: format!("{base}/{id}"),
        bundle,
        bundle_id: id,
        runner_args,
        tests: m.tests.iter().map(|t| t.name.clone()).collect(),
    };
    util::write_json(&stage.join("plan.json"), &plan)?;
    util::write(&stage.join("transfer.sh"), transfer_script(&plan))?;
    println!(
        "bundle {}: {} tests, {} bytes, staged in {}",
        plan.bundle_id,
        plan.tests.len(),
        text.len(),
        stage.display()
    );
    Ok(plan)
}

/// Check a fetched result file against the stage: the bundle it ran, the
/// executables, every test there. Returns problems (empty: usable).
pub fn check_results(plan: &Plan, m: &Manifest, r: &RunFile) -> Vec<String> {
    let mut problems = Vec::new();
    if r.system.name != "windows" {
        problems.push(format!("the results are from {:?}, not windows", r.system.name));
    }
    if r.bundle.as_deref() != Some(plan.bundle_id.as_str()) {
        problems.push(format!("the results are of bundle {:?}, not {}", r.bundle, plan.bundle_id));
    }
    for t in &m.tests {
        match r.test(&t.name) {
            None => problems.push(format!("{}: no result", t.name)),
            Some(x) if x.exe_sha256.as_deref() != Some(t.exe_sha256.as_str()) => {
                problems.push(format!("{}: ran a different executable", t.name))
            }
            Some(_) => {}
        }
    }
    problems
}

/// A result's verdict for people: why it cannot be a reference, if so.
fn unusable(t: &windows::RunResult) -> Option<String> {
    match (t.status.as_str(), t.stable) {
        ("ran", Some(false)) => Some("not stable between runs".into()),
        ("ran", _) => None,
        (s, _) => Some(format!("{s}{}", t.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default())),
    }
}

/// The raw result file as JSON (all of the runner's fields kept, a UTF-8
/// byte-order mark allowed) with only `tests` named.
fn subset(raw: &serde_json::Value, names: &[&str]) -> serde_json::Value {
    let mut v = raw.clone();
    if let Some(tests) = v.get_mut("tests").and_then(|t| t.as_array_mut()) {
        tests.retain(|t| t.get("name").and_then(|n| n.as_str()).is_some_and(|n| names.contains(&n)));
    }
    v
}

/// Merge `new` (one run's tests) into the reference file `old`: its tests
/// replace those of the same name, the file's other fields are `new`'s.
fn merged(old: Option<serde_json::Value>, new: serde_json::Value) -> serde_json::Value {
    let Some(old) = old else { return new };
    let mut out = new.clone();
    let name = |t: &serde_json::Value| t.get("name").and_then(|n| n.as_str()).map(String::from);
    let fresh: Vec<String> =
        new.get("tests").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(name).collect()).unwrap_or_default();
    let mut tests: Vec<serde_json::Value> = old
        .get("tests")
        .and_then(|t| t.as_array())
        .map(|a| a.iter().filter(|t| name(t).is_none_or(|n| !fresh.contains(&n))).cloned().collect())
        .unwrap_or_default();
    tests.extend(new.get("tests").and_then(|t| t.as_array()).cloned().unwrap_or_default());
    tests.sort_by_key(name);
    out["tests"] = serde_json::Value::Array(tests);
    out
}

/// Where a test's Windows reference is kept (ADR 0075, 0097): the corpus's
/// in `tests/windows/oracle/<test>.windows.json`, a spec entry's (or the
/// held-out store's) in `windows.json` beside the test.
pub fn reference_path(repo: &Path, test: &str, source: &str) -> PathBuf {
    let src = if Path::new(source).is_absolute() { PathBuf::from(source) } else { repo.join(source) };
    match src.parent() {
        Some(dir) if dir == repo.join(windows::CORPUS_DIR) => {
            repo.join(ORACLE_DIR).join(format!("{test}.windows.json"))
        }
        Some(dir) => dir.join("windows.json"),
        None => repo.join(ORACLE_DIR).join(format!("{test}.windows.json")),
    }
}

/// Record the usable results as references; returns the files written.
fn record(repo: &Path, m: &Manifest, raw: &serde_json::Value, results: &RunFile) -> Result<Vec<PathBuf>> {
    let mut by_file: std::collections::BTreeMap<PathBuf, Vec<&str>> = Default::default();
    for t in &m.tests {
        let Some(r) = results.test(&t.name) else { continue };
        if let Some(why) = unusable(r) {
            println!("not recorded: {} ({why})", t.name);
            continue;
        }
        by_file.entry(reference_path(repo, &t.name, &t.source)).or_default().push(&t.name);
    }
    let mut written = Vec::new();
    for (path, names) in by_file {
        let old = if path.exists() && !path.starts_with(repo.join(ORACLE_DIR)) {
            let bytes = util::read(&path)?;
            let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
            Some(serde_json::from_slice(bytes).map_err(|e| format!("{}: {e}", path.display()))?)
        } else {
            None
        };
        let value = merged(old, subset(raw, &names));
        util::write_json(&path, &value)?;
        written.push(path);
    }
    Ok(written)
}

/// Read a fetched stage's results, print them, `--record` and `--diff`.
fn finish(repo: &Path, a: &Args, stage: &Path) -> Result<u8> {
    crate::oracle_guard("win remote finish")?;
    let plan: Plan = util::read_json(&stage.join("plan.json"))?;
    if plan.format != PLAN_FORMAT {
        return Err(format!("{}: not a {PLAN_FORMAT} stage", stage.display()));
    }
    let m: Manifest = util::read_json(&stage.join("manifest.json"))?;
    let path = stage.join("results.json");
    let bytes = util::read(&path).map_err(|e| format!("{e} (has transfer.sh run?)"))?;
    let results = RunFile::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let raw: serde_json::Value = serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let s = &results.system;
    println!(
        "{}: {} {} {}, bundle {}",
        plan.host,
        s.name,
        s.version.as_deref().unwrap_or(""),
        s.build.as_deref().unwrap_or(""),
        results.bundle.as_deref().unwrap_or("?")
    );
    for t in &results.tests {
        let code = t.exit_code.map_or("none".into(), |c| format!("{c:#x}"));
        let why = unusable(t).map(|w| format!("  ({w})")).unwrap_or_default();
        println!("  {:<28} {:<8} exit {code}{why}", t.name, t.status);
    }
    let problems = check_results(&plan, &m, &results);
    for p in &problems {
        println!("problem: {p}");
    }
    if !problems.is_empty() {
        return Err(format!("{}: {} problems; not using these results", path.display(), problems.len()));
    }
    let keep = repo.join(windows::WIN_DIR).join("results").join(format!("windows-{}.json", plan.bundle_id));
    util::write(&keep, &bytes)?;
    println!("results: {}", keep.display());
    if a.has("record") {
        for f in record(repo, &m, &raw, &results)? {
            println!("recorded: {}", f.strip_prefix(repo).unwrap_or(&f).display());
        }
    }
    if !a.has("diff") {
        return Ok(u8::from(results.tests.iter().any(|t| unusable(t).is_some())));
    }
    // Poly runs exactly the executables Windows ran: the stage's bundle
    // (whose hashes `unbundle` checks).
    let text = util::read_string(&stage.join(&plan.bundle))?;
    let (bm, files) = windows::unbundle(&text)?;
    let bin = stage.join("bin");
    for (name, bytes) in &files {
        util::write(&bin.join(name), bytes)?;
    }
    util::write_json(&bin.join("manifest.json"), &bm)?;
    let kernel = a.opt("kernel").map_or_else(|| poly::default_kernel(repo), PathBuf::from);
    if a.has("build-kernel") || (a.opt("kernel").is_none() && !kernel.exists()) {
        poly::build_kernel(repo)?;
    }
    let target = poly::Target { kernel, timeout: crate::timeout(a, crate::POLY_TIMEOUT)? };
    let jobs: usize = a.opt("jobs").map_or(Ok(1), str::parse).map_err(|_| "--jobs: a number")?;
    let run_dir = repo.join(crate::RUNS_DIR).join(format!("{}-windows-remote", util::now_stamp()));
    let poly_results = windows::run_poly(&target, &bin, &bm, &run_dir, jobs.max(1))?;
    util::write_json(&run_dir.join("poly.json"), &poly_results)?;
    let summary = windows::diff(&results, &poly_results, &[])?;
    let text = windows::report(&summary);
    util::write(&run_dir.join("report.txt"), &text)?;
    util::write_json(&run_dir.join("summary.json"), &summary)?;
    print!("{text}");
    println!("run directory: {}", run_dir.display());
    let bad = [windows::Verdict::Fail, windows::Verdict::Missing, windows::Verdict::Error]
        .iter()
        .map(|&v| summary.count(v))
        .sum::<usize>()
        + if a.has("strict") { summary.count(windows::Verdict::Unsupported) } else { 0 };
    Ok(u8::from(bad > 0))
}

const OPTIONS: &[&str] = &[
    "host",
    "remote-dir",
    "runner-arg",
    "tests",
    "tests-dir",
    "stage",
    "record",
    "diff",
    "kernel",
    "build-kernel",
    "timeout",
    "jobs",
    "strict",
];

/// `win remote [prepare|finish] ...`.
pub fn cmd(repo: &Path, a: &Args, rest: &[String]) -> Result<u8> {
    a.only(OPTIONS)?;
    let stage = stage_dir(repo, a)?;
    match rest {
        [p] if p == "prepare" => {
            let plan = prepare(repo, a, &stage)?;
            println!("next, where `ssh {}` works: sh {}/transfer.sh", plan.host, stage.display());
            Ok(0)
        }
        [f] if f == "finish" => finish(repo, a, &stage),
        [] => {
            crate::oracle_guard("win remote")?;
            let plan = prepare(repo, a, &stage)?;
            let script = stage.join("transfer.sh");
            if !ssh_usable() {
                let rel = stage.strip_prefix(repo).unwrap_or(&stage).display().to_string();
                println!("no SSH here (a container?); on the host, from the repository root:");
                println!("  sh {rel}/transfer.sh");
                println!("then here: cargo xtask pipeline win remote finish --stage {rel} [--record] [--diff]");
                println!("(tools/pipeline/windows/poly-remote.sh does all three from the host)");
                return Ok(2);
            }
            let status = Command::new("sh").arg(&script).status().map_err(|e| format!("sh: {e}"))?;
            if !stage.join("results.json").exists() {
                return Err(format!(
                    "{} failed ({status}) and fetched no results from {}",
                    script.display(),
                    plan.host
                ));
            }
            finish(repo, a, &stage)
        }
        _ => {
            Err("usage: pipeline win remote [prepare|finish] --host H [options] (see tools/pipeline/src/main.rs)"
                .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            format: PLAN_FORMAT.into(),
            host: "homeserver2".into(),
            remote_dir: "poly-oracle-work/remote/0123456789ab".into(),
            bundle: "bundle-0123456789ab.txt".into(),
            bundle_id: "0123456789ab".into(),
            runner_args: vec!["-Runs".into(), "3".into()],
            tests: vec!["a".into(), "b".into()],
        }
    }

    #[test]
    fn validates_what_reaches_a_shell() {
        assert!(valid_host("homeserver2") && valid_host("server@10.0.0.2") && valid_host("my-host.lan"));
        assert!(!valid_host("-oProxyCommand=x") && !valid_host("a b") && !valid_host("a;b") && !valid_host(""));
        assert!(valid_remote_dir("poly-oracle-work/remote") && valid_remote_dir("C:/work/poly"));
        assert!(!valid_remote_dir("../x") && !valid_remote_dir("a/../b") && !valid_remote_dir("/abs"));
        assert!(
            !valid_remote_dir("a'b")
                && !valid_remote_dir("a b")
                && !valid_remote_dir("a$b")
                && !valid_remote_dir("a//b")
        );
        assert!(valid_runner_arg("-TestTimeouts") && valid_runner_arg("sevenzip_scenario=300,b=10"));
        assert!(!valid_runner_arg("x;y") && !valid_runner_arg("$(x)") && !valid_runner_arg("'"));
    }

    #[test]
    fn encodes_powershell_commands() {
        // UTF-16LE "ab" = 61 00 62 00.
        assert_eq!(encode_command("ab"), "YQBiAA==");
        assert!(
            encode_command("Set-Location -LiteralPath $env:USERPROFILE")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        );
    }

    #[test]
    fn transfer_script_runs_copies_and_cleans() {
        let p = plan();
        let [mkdir, run, clean] = remote_scripts(&p);
        assert!(
            mkdir.contains(
                "New-Item -ItemType Directory -Force -Path 'poly-oracle-work/remote/0123456789ab' | Out-Null"
            )
        );
        assert!(run.contains(
            "& 'poly-oracle-work/remote/0123456789ab/poly-oracle.ps1' -Bundle 'poly-oracle-work/remote/0123456789ab/bundle-0123456789ab.txt' -Out 'poly-oracle-work/remote/0123456789ab/results.json' -Work 'poly-oracle-work/remote/0123456789ab/work' -Runs 3 6>&1"
        ));
        for s in [&mkdir, &run, &clean] {
            assert!(s.starts_with(
                "$ProgressPreference = 'SilentlyContinue'; $ErrorActionPreference = 'Stop'; try { Set-Location"
            ));
            assert!(s.ends_with("catch { [Console]::Error.WriteLine([string]$_); exit 1 }"));
        }
        assert!(clean.contains("Remove-Item -LiteralPath 'poly-oracle-work/remote/0123456789ab' -Recurse -Force"));
        let s = transfer_script(&p);
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("scp -q -o BatchMode=yes bundle-0123456789ab.txt poly-oracle.ps1 homeserver2:poly-oracle-work/remote/0123456789ab/\n"));
        assert!(s.contains(
            "scp -q -o BatchMode=yes homeserver2:poly-oracle-work/remote/0123456789ab/results.json results.json"
        ));
        assert!(s.contains(&format!("remote {} || status=$?\n", encode_command(&run))));
        assert!(s.contains("-ExecutionPolicy Bypass -EncodedCommand"));
        // Copy, run, fetch, clean, in that order.
        let at = |needle: &str| s.find(needle).unwrap();
        assert!(at(&encode_command(&mkdir)) < at("scp -q -o BatchMode=yes bundle"));
        assert!(at("scp -q -o BatchMode=yes bundle") < at(&encode_command(&run)));
        assert!(at(&encode_command(&run)) < at("results.json results.json"));
        assert!(at("results.json results.json") < at(&encode_command(&clean)));
    }

    fn built(name: &str, hash: &str, source: &str) -> windows::Built {
        windows::Built {
            name: name.into(),
            source: source.into(),
            source_sha256: String::new(),
            exe: format!("{name}.exe"),
            exe_sha256: hash.into(),
            size: 0,
            imports: Default::default(),
        }
    }

    fn results_json() -> &'static str {
        r#"{"format":"poly-winrun/1","system":{"name":"windows","version":"10.0.26200.0","build":"26200.1","arch":"AMD64","edition":"Core"},
            "bundle":"0123456789ab","runner":"poly-oracle.ps1 1","runs":2,
            "tests":[{"name":"a","exe_sha256":"aa","status":"ran","exit_code":0,"output":"ok x\n","stable":true},
                     {"name":"b","exe_sha256":"bb","status":"error","exit_code":null,"error":"could not start: denied","stable":true}]}"#
    }

    #[test]
    fn checks_results_against_the_stage() {
        let p = plan();
        let m = Manifest {
            format: windows::BUILD_FORMAT.into(),
            rustc: String::new(),
            revision: None,
            tests: vec![built("a", "aa", "tests/windows/a.rs"), built("b", "bb", "tests/windows/b.rs")],
        };
        let r = RunFile::parse(results_json().as_bytes()).unwrap();
        assert!(check_results(&p, &m, &r).is_empty());
        let mut other = p.clone();
        other.bundle_id = "ffffffffffff".into();
        assert_eq!(check_results(&other, &m, &r).len(), 1);
        let mut m2 = m.clone();
        m2.tests[0].exe_sha256 = "zz".into();
        m2.tests.push(built("c", "cc", "tests/windows/c.rs"));
        assert_eq!(check_results(&p, &m2, &r), ["a: ran a different executable", "c: no result"]);
        assert_eq!(unusable(r.test("a").unwrap()), None);
        assert_eq!(unusable(r.test("b").unwrap()).as_deref(), Some("error: could not start: denied"));
    }

    #[test]
    fn references_go_where_adr_0075_says_and_merge_per_test() {
        let repo = Path::new("/repo");
        assert_eq!(
            reference_path(repo, "nt_file_rw", "tests/windows/nt_file_rw.rs"),
            Path::new("/repo/tests/windows/oracle/nt_file_rw.windows.json")
        );
        assert_eq!(
            reference_path(repo, "t1", "specs/windows/ntdll/NtClose.tests/t1.rs"),
            Path::new("/repo/specs/windows/ntdll/NtClose.tests/windows.json")
        );
        assert_eq!(
            reference_path(repo, "t2", "/heldout/windows/ntdll/NtClose.tests/t2.rs"),
            Path::new("/heldout/windows/ntdll/NtClose.tests/windows.json")
        );
        let raw: serde_json::Value = serde_json::from_str(results_json()).unwrap();
        let only_a = subset(&raw, &["a"]);
        assert_eq!(only_a["tests"].as_array().unwrap().len(), 1);
        assert_eq!(only_a["system"]["edition"], "Core");
        // A newer run of `a` replaces it; `b` from before stays.
        let mut newer = only_a.clone();
        newer["tests"][0]["output"] = "ok y\n".into();
        newer["bundle"] = "newbundle".into();
        let m = merged(Some(raw.clone()), newer);
        let names: Vec<&str> = m["tests"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(m["tests"][0]["output"], "ok y\n");
        assert_eq!(m["bundle"], "newbundle");
        assert_eq!(merged(None, only_a.clone()), only_a);
    }
}
