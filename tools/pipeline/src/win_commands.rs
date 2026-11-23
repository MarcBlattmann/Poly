//! The Windows test oracle commands: win polycheck, win (build, bundle, poly, diff, show, remote) and win scenario.

use super::*;

/// Poly against the recorded Windows results of every green Windows
/// entry's visible tests (ADR 0099; ADR 0098 left it to be built): each
/// must pass, so a green entry stays green. Skipped, successfully, without
/// green Windows entries, without QEMU, without clang++ when an entry has
/// C++ tests, and for implementer agents (it reads Windows' observations).
/// With `--shard I/N` it builds and checks only the tests in slice I of N
/// ([`stable_slice`]), the vcruntime140 parity pass included: every test is
/// in exactly one slice.
fn win_polycheck(repo: &Path, a: &Args) -> Result<u8> {
    let shard = a.opt("shard").map(parse_shard).transpose()?;
    if oracle_guard("win polycheck").is_err() {
        println!("pipeline win polycheck: skipped (implementer agents do not read Windows results)");
        return Ok(0);
    }
    let mut entries = green::windows_entries(repo)?;
    if entries.is_empty() {
        println!("pipeline win polycheck: no green Windows entry has visible tests with recorded results");
        return Ok(0);
    }
    if Command::new("qemu-system-x86_64").arg("--version").output().is_err() {
        println!("pipeline win polycheck: skipped (no qemu-system-x86_64)");
        return Ok(0);
    }
    let has_cpp = entries
        .iter()
        .any(|w| w.entry.tests.iter().any(|t| repo.join(&w.entry.tests_dir).join(format!("{t}.cpp")).is_file()));
    if has_cpp && let Err(e) = wincxx::Tools::from_env().version() {
        println!("pipeline win polycheck: skipped (C++ tests need clang++, lld-link and llvm-dlltool: {e})");
        return Ok(0);
    }
    // Whether the parity pass runs at all is decided on every entry, so each
    // slice runs its part of it.
    let parity_wanted = entries.iter().any(|w| {
        w.entry.tests_dir.starts_with(MSVCRT_SPECS)
            && w.entry.tests.iter().any(|t| repo.join(&w.entry.tests_dir).join(format!("{t}.cpp")).is_file())
    });
    let in_shard = |name: &str| shard.is_none_or(|(i, n)| stable_slice(name, n) == i);
    if let Some((i, n)) = shard {
        let total: usize = entries.iter().map(|w| w.entry.tests.len()).sum();
        for w in &mut entries {
            w.entry.tests.retain(|t| in_shard(t));
        }
        entries.retain(|w| !w.entry.tests.is_empty());
        let mine: usize = entries.iter().map(|w| w.entry.tests.len()).sum();
        println!("pipeline win polycheck: shard {i}/{n}: {mine} of the {total} tests");
    }
    let base = repo.join(windows::WIN_DIR).join("polycheck");
    let bin = base.join("bin");
    let dirs: Vec<PathBuf> = entries.iter().map(|w| repo.join(&w.entry.tests_dir)).collect();
    let tests: Vec<String> = entries.iter().flat_map(|w| w.entry.tests.iter().cloned()).collect();
    let own: Vec<String> = VCRT_TESTS
        .iter()
        .filter(|t| parity_wanted && in_shard(t) && repo.join(windows::CORPUS_DIR).join(format!("{t}.cpp")).is_file())
        .map(|t| t.to_string())
        .collect();
    if tests.is_empty() && own.is_empty() {
        println!("pipeline win polycheck: no test in this slice");
        return Ok(0);
    }
    let kernel = match a.opt("kernel") {
        Some(k) => PathBuf::from(k),
        None => {
            poly::build_kernel(repo)?;
            poly::default_kernel(repo)
        }
    };
    let target = poly::Target { kernel, timeout: timeout(a, POLY_TIMEOUT)? };
    let run_dir = base.join("run");
    let _ = std::fs::remove_dir_all(&run_dir);
    let jobs = match a.opt("jobs") {
        Some(j) => j.parse().map_err(|_| "--jobs: a number")?,
        None => util::default_jobs(POLYCHECK_MAX_JOBS),
    };
    let mut report = String::new();
    let mut problems = Vec::new();
    // A slice may hold only parity tests: an empty list would build them all.
    let results = if tests.is_empty() {
        windows::RunFile::default()
    } else {
        let manifest = windows::build(repo, &bin, &dirs, &tests)?;
        windows::run_poly(&target, &bin, &manifest, &run_dir, jobs)?
    };
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
    util::write_json(&run_dir.join("poly.json"), &results)?;
    for w in &entries {
        let reference = windows::RunFile::load(&repo.join(&w.reference))?;
        let summary = windows::diff(&reference, &results, &w.entry.tests)?;
        report.push_str(&format!("== {}\n{}\n", w.entry.id, windows::report(&summary)));
        let mut bad: Vec<String> =
            summary.tests.iter().filter(|t| t.verdict != windows::Verdict::Pass).map(|t| t.name.clone()).collect();
        bad.extend(
            w.entry.tests.iter().filter(|t| reference.test(t).is_none()).map(|t| format!("{t} (no recorded result)")),
        );
        if !bad.is_empty() {
            problems.push(format!("green entry {}: {}", w.entry.id, bad.join(", ")));
        }
    }
    let parity = vcruntime_parity(repo, &entries, &own, &base, &target, jobs, &mut report, &mut problems)?;
    util::write(&run_dir.join("report.txt"), &report)?;
    print!("{report}");
    if !problems.is_empty() {
        return Err(format!("on Poly, {} (report: {})", problems.join("; "), run_dir.join("report.txt").display()));
    }
    println!(
        "pipeline win polycheck: the {} visible tests of {} green Windows entries pass on Poly \
         ({parity} of them also with vcruntime140.dll)",
        tests.len(),
        entries.len()
    );
    Ok(0)
}

/// The tests directory of a green `msvcrt.dll` entry (ADR 0199's parity pass).
const MSVCRT_SPECS: &str = "specs/windows/msvcrt.dll";
/// The corpus tests of `vcruntime140.dll`'s own exports (ADR 0199), in `tests/windows`.
const VCRT_TESTS: &[&str] = &["vcrt_exception_state", "vcrt_typeinfo_exception", "vcrt_noexcept_destructor"];

/// ADR 0199: `vcruntime140.dll` runs the C++ exception code `msvcrt.dll` has (ADR 0099), so
/// the C++ tests of the green `msvcrt.dll` entries, built to take their runtime from
/// `vcruntime140.dll` (`wincxx::Runtime::Vcruntime140`), must give the results recorded for
/// them on Windows. Until a spec agent records `vcruntime140.dll`'s own results, this is
/// the check that the two DLLs behave alike. The implementer's own `vcruntime140.dll` tests
/// ([`VCRT_TESTS`], self-checking, no Windows results yet) run in the same boot and must pass.
/// Returns how many tests ran.
#[allow(clippy::too_many_arguments)]
fn vcruntime_parity(
    repo: &Path,
    entries: &[green::WindowsEntry],
    own: &[String],
    base: &Path,
    target: &poly::Target,
    jobs: usize,
    report: &mut String,
    problems: &mut Vec<String>,
) -> Result<usize> {
    let cpp = |w: &green::WindowsEntry| -> Vec<String> {
        w.entry
            .tests
            .iter()
            .filter(|t| repo.join(&w.entry.tests_dir).join(format!("{t}.cpp")).is_file())
            .cloned()
            .collect()
    };
    let chosen: Vec<(&green::WindowsEntry, Vec<String>)> = entries
        .iter()
        .filter(|w| w.entry.tests_dir.starts_with(MSVCRT_SPECS))
        .map(|w| (w, cpp(w)))
        .filter(|(_, t)| !t.is_empty())
        .collect();
    if chosen.is_empty() && own.is_empty() {
        return Ok(0);
    }
    let bin = base.join("bin-vcruntime140");
    let dirs: Vec<PathBuf> = chosen.iter().map(|(w, _)| repo.join(&w.entry.tests_dir)).collect();
    let tests: Vec<String> = chosen.iter().flat_map(|(_, t)| t.iter().cloned()).chain(own.iter().cloned()).collect();
    let manifest = windows::build_runtime(repo, &bin, &dirs, &tests, Some(wincxx::Runtime::Vcruntime140))?;
    let run_dir = base.join("run-vcruntime140");
    let _ = std::fs::remove_dir_all(&run_dir);
    let mut results = windows::run_poly(target, &bin, &manifest, &run_dir, jobs)?;
    util::write_json(&run_dir.join("poly.json"), &results)?;
    // The reference ran the msvcrt build of each test: these are other executables on
    // purpose, so only what they observed is compared.
    for r in &mut results.tests {
        r.exe_sha256 = None;
    }
    for (w, t) in &chosen {
        let reference = windows::RunFile::load(&repo.join(&w.reference))?;
        let summary = windows::diff(&reference, &results, t)?;
        report.push_str(&format!("== {} (built for vcruntime140.dll)\n{}\n", w.entry.id, windows::report(&summary)));
        let bad: Vec<String> =
            summary.tests.iter().filter(|t| t.verdict != windows::Verdict::Pass).map(|t| t.name.clone()).collect();
        if !bad.is_empty() {
            problems.push(format!("{} built for vcruntime140.dll: {}", w.entry.id, bad.join(", ")));
        }
    }
    for name in own {
        let r = results.test(name);
        let ok = r.is_some_and(|r| r.exit_code == Some(0));
        report.push_str(&format!(
            "== {name} (vcruntime140.dll, self-checking): {}
",
            if ok { "pass" } else { "FAIL" }
        ));
        if let Some(out) = r.and_then(|r| r.output.as_deref()) {
            for line in out.lines().filter(|l| l.starts_with("not ok") || l.starts_with("summary")) {
                report.push_str(&format!(
                    "   {line}
"
                ));
            }
        }
        if !ok {
            problems.push(format!("{name} on vcruntime140.dll (exit {:?})", r.and_then(|r| r.exit_code)));
        }
    }
    Ok(tests.len())
}

/// The freestanding hello world makes exactly two syscalls.
pub(super) fn check_hello(t: &Trace) -> Result {
    let lines: Vec<String> = trace::normalize(t, &NormOptions::default()).iter().map(ToString::to_string).collect();
    let want = ["write(1, \"hello, world\\n\", 13) = 13", "exit_group(0) = ?"];
    if lines != want || t.stdout != "hello, world\n" || t.exit != trace::Exit::Code(0) {
        return Err(format!("unexpected hello trace: {lines:?}, stdout {:?}, {}", t.stdout, t.exit));
    }
    Ok(())
}

/// The Windows test oracle (ADR 0075): `win build|bundle|poly|diff|show`.
pub(super) fn win_cmd(repo: &Path, a: &Args) -> Result<u8> {
    let usage =
        "usage: pipeline win <build|bundle|unbundle|poly|diff|show|remote|polycheck> (see tools/pipeline/src/main.rs)";
    let (sub, rest) = a.positional.split_first().ok_or(usage)?;
    let win = repo.join(windows::WIN_DIR);
    let bin_dir = a.opt("bin-dir").map_or_else(|| win.join("bin"), PathBuf::from);
    match sub.as_str() {
        "build" => {
            a.only(&["tests-dir", "tests", "out", "cxx-runtime"])?;
            let out = a.opt("out").map_or_else(|| win.join("bin"), PathBuf::from);
            let extra: Vec<PathBuf> = a.all("tests-dir").into_iter().map(PathBuf::from).collect();
            let runtime = a.opt("cxx-runtime").map(wincxx::Runtime::parse).transpose()?;
            let m = windows::build_runtime(repo, &out, &extra, &test_list(a), runtime)?;
            for t in &m.tests {
                let imports: Vec<String> = t.imports.iter().map(|(d, n)| format!("{d}: {}", n.join(" "))).collect();
                println!("{:<28} {:>6} bytes  {}", t.exe, t.size, imports.join("; "));
            }
            println!("built {} Windows test programs into {}", m.tests.len(), out.display());
            Ok(0)
        }
        "bundle" => {
            a.only(&["bin-dir", "out", "part-size", "data-dir"])?;
            let m = windows::Manifest::load(&bin_dir)?;
            let data = match a.opt("data-dir") {
                Some(d) => windows::data_files(&m, Path::new(d))?,
                None => Vec::new(),
            };
            for (test, name, bytes) in &data {
                println!("data {test}/{name}: {} bytes", bytes.len());
            }
            let text = windows::bundle_with_data(&m, &bin_dir, &data)?;
            let id = windows::bundle_id(&text);
            let out = a.opt("out").map_or_else(|| win.join(format!("bundle-{id}.txt")), PathBuf::from);
            util::write(&out, &text)?;
            println!("bundle {id}: {} tests, {} bytes: {}", m.tests.len(), text.len(), out.display());
            let limit: usize = a.opt("part-size").map_or(Ok(45_000), str::parse).map_err(|_| "--part-size: bytes")?;
            let parts = windows::split_parts(&text, limit);
            if parts.len() > 1 {
                for (i, p) in parts.iter().enumerate() {
                    let path = PathBuf::from(format!("{}.part{}of{}", out.display(), i + 1, parts.len()));
                    util::write(&path, p)?;
                    println!("  part {}: {} bytes: {}", i + 1, p.len(), path.display());
                }
            }
            Ok(0)
        }
        "poly" | "diff" => {
            a.only(&[
                "bin-dir",
                "kernel",
                "build-kernel",
                "timeout",
                "jobs",
                "tests",
                "out",
                "reference",
                "target",
                "strict",
            ])?;
            let only = test_list(a);
            let reference = match a.opt("reference") {
                Some(path) => {
                    oracle_guard("win diff")?;
                    Some(windows::RunFile::load(Path::new(path))?)
                }
                None if sub == "diff" => return Err("win diff needs --reference <results.json from Windows>".into()),
                None => None,
            };
            let run_dir = repo.join(RUNS_DIR).join(format!(
                "{}-windows-{}",
                util::now_stamp(),
                if sub == "poly" { "poly" } else { "diff" }
            ));
            let target = match a.opt("target") {
                Some(path) if path != "poly" => windows::RunFile::load(Path::new(path))?,
                _ => {
                    let kernel = a.opt("kernel").map_or_else(|| poly::default_kernel(repo), PathBuf::from);
                    if a.has("build-kernel") || (a.opt("kernel").is_none() && !kernel.exists()) {
                        poly::build_kernel(repo)?;
                    }
                    let target = poly::Target { kernel, timeout: timeout(a, POLY_TIMEOUT)? };
                    let mut m = windows::Manifest::load(&bin_dir)?;
                    if !only.is_empty() {
                        m.tests.retain(|t| only.contains(&t.name));
                    }
                    let jobs: usize = a.opt("jobs").map_or(Ok(1), str::parse).map_err(|_| "--jobs: a number")?;
                    let results = windows::run_poly(&target, &bin_dir, &m, &run_dir, jobs.max(1))?;
                    util::write_json(&run_dir.join("poly.json"), &results)?;
                    results
                }
            };
            let Some(reference) = reference else {
                for t in &target.tests {
                    let code = t.exit_code.map_or("none".into(), |c| format!("{c:#x}"));
                    println!(
                        "== {} {} exit {code}{}",
                        t.name,
                        t.status,
                        t.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
                    );
                    print!("{}", t.output.as_deref().unwrap_or_default());
                }
                if let Some(out) = a.opt("out") {
                    util::write_json(Path::new(out), &target)?;
                }
                println!("results: {}", run_dir.join("poly.json").display());
                return Ok(0);
            };
            let summary = windows::diff(&reference, &target, &only)?;
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
        "polycheck" => {
            a.only(&["kernel", "jobs", "timeout", "shard"])?;
            win_polycheck(repo, a)
        }
        "unbundle" => {
            // What the Windows runner does with a bundle, here: check it
            // and write its files (to verify a transfer, or to look inside).
            a.only(&["out"])?;
            if rest.is_empty() {
                return Err("usage: pipeline win unbundle <bundle part>... --out DIR".into());
            }
            let mut text = String::new();
            for p in rest {
                text.push_str(&util::read_string(Path::new(p))?);
            }
            let (m, files, data) = windows::unbundle_all(&text)?;
            let out = PathBuf::from(a.require("out")?);
            for (name, bytes) in &files {
                util::write(&out.join(name), bytes)?;
            }
            for (test, name, bytes) in &data {
                util::write(&out.join("data").join(test).join(name), bytes)?;
            }
            util::write_json(&out.join("manifest.json"), &m)?;
            println!(
                "bundle {}: {} files and {} data files, hashes good, into {}",
                windows::bundle_id(&text),
                files.len(),
                data.len(),
                out.display()
            );
            Ok(0)
        }
        "show" => {
            oracle_guard("win show")?;
            a.only(&["tests"])?;
            let [path] = rest else { return Err("usage: pipeline win show <results.json> [--tests a,b]".into()) };
            let f = windows::RunFile::load(Path::new(path))?;
            let only = test_list(a);
            println!(
                "{} {} {}",
                f.system.name,
                f.system.version.as_deref().unwrap_or(""),
                f.system.build.as_deref().unwrap_or("")
            );
            for t in f.tests.iter().filter(|t| only.is_empty() || only.contains(&t.name)) {
                let code = t.exit_code.map_or("none".into(), |c| format!("{c:#x}"));
                let stable = if t.stable == Some(false) { " (NOT STABLE between runs)" } else { "" };
                println!(
                    "== {} {} exit {code}{stable}{}",
                    t.name,
                    t.status,
                    t.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
                );
                print!("{}", t.output.as_deref().unwrap_or_default());
            }
            Ok(0)
        }
        "scenario" => win_scenario(repo, a, rest, &bin_dir),
        "remote" => remote::cmd(repo, a, rest),
        _ => Err(usage.into()),
    }
}

/// `win scenario poly|diff|show` (ADR 0088): scenario drivers, which run
/// an unchanged application step by step, on Poly and compared with a
/// Windows recording step by step.
fn win_scenario(repo: &Path, a: &Args, rest: &[String], bin_dir: &Path) -> Result<u8> {
    let usage = "usage: pipeline win scenario <poly|diff|show> (see tools/pipeline/src/main.rs)";
    let (sub, rest) = rest.split_first().ok_or(usage)?;
    let only = test_list(a);
    match sub.as_str() {
        "show" => {
            oracle_guard("win scenario show")?;
            a.only(&["tests"])?;
            let [path] = rest else {
                return Err("usage: pipeline win scenario show <results.json> [--tests a,b]".into());
            };
            let f = windows::RunFile::load(Path::new(path))?;
            for t in scenario::scenario_tests(&f).into_iter().filter(|t| only.is_empty() || only.contains(&t.name)) {
                let bytes = windows::unbase64(t.output_b64.as_deref().unwrap_or_default())?;
                println!("== {} on {} (status {}, stable {:?})", t.name, f.system.name, t.status, t.stable);
                print!("{}", scenario::show(&scenario::parse(&bytes)?));
            }
            Ok(0)
        }
        "poly" | "diff" => {
            a.only(&[
                "bin-dir",
                "data-dir",
                "kernel",
                "build-kernel",
                "timeout",
                "tests",
                "out",
                "reference",
                "target",
                "markdown",
            ])?;
            let reference = match a.opt("reference") {
                Some(path) => {
                    oracle_guard("win scenario diff")?;
                    Some((path.to_string(), windows::RunFile::load(Path::new(path))?))
                }
                None if sub == "diff" => {
                    return Err("win scenario diff needs --reference <results.json from Windows>".into());
                }
                None => None,
            };
            let run_dir = repo.join(RUNS_DIR).join(format!("{}-scenario-{sub}", util::now_stamp()));
            let target = match a.opt("target") {
                Some(path) if path != "poly" => windows::RunFile::load(Path::new(path))?,
                _ => {
                    let kernel = a.opt("kernel").map_or_else(|| poly::default_kernel(repo), PathBuf::from);
                    if a.has("build-kernel") || (a.opt("kernel").is_none() && !kernel.exists()) {
                        poly::build_kernel(repo)?;
                    }
                    let target = poly::Target { kernel, timeout: timeout(a, SCENARIO_TIMEOUT)? };
                    let mut m = windows::Manifest::load(bin_dir)?;
                    // The scenarios: the tests the reference has as scenarios, or those named.
                    let wanted: Vec<String> = match (&reference, only.is_empty()) {
                        (_, false) => only.clone(),
                        (Some((_, r)), true) => scenario::scenario_tests(r).iter().map(|t| t.name.clone()).collect(),
                        (None, true) => {
                            m.tests.iter().filter(|t| t.name.ends_with("_scenario")).map(|t| t.name.clone()).collect()
                        }
                    };
                    m.tests.retain(|t| wanted.contains(&t.name));
                    if m.tests.is_empty() {
                        return Err(format!(
                            "no scenario drivers to run in {} (build them: win build)",
                            bin_dir.display()
                        ));
                    }
                    let data_dir = a.opt("data-dir").map_or_else(|| repo.join(scenario::DATA_DIR), PathBuf::from);
                    let results = scenario::run_poly(&target, bin_dir, &m, &data_dir, &run_dir)?;
                    util::write_json(&run_dir.join("poly.json"), &results)?;
                    results
                }
            };
            if let Some(out) = a.opt("out") {
                util::write_json(Path::new(out), &target)?;
            }
            let Some((reference_path, reference)) = reference else {
                for t in &target.tests {
                    let code = t.exit_code.map_or("none".into(), |c| format!("{c:#x}"));
                    println!(
                        "== {} {} exit {code}{}",
                        t.name,
                        t.status,
                        t.error.as_deref().map(|e| format!(": {e}")).unwrap_or_default()
                    );
                    let bytes = windows::unbase64(t.output_b64.as_deref().unwrap_or_default())?;
                    if scenario::is_scenario(&bytes) {
                        print!("{}", scenario::show(&scenario::parse(&bytes)?));
                    }
                }
                println!("results: {}", run_dir.join("poly.json").display());
                return Ok(0);
            };
            let mut all_passed = true;
            let mut reports = Vec::new();
            for r in scenario::scenario_tests(&reference) {
                if !only.is_empty() && !only.contains(&r.name) {
                    continue;
                }
                let report = scenario::compare(r, target.test(&r.name), (&reference.system, &target.system))?;
                all_passed &= report.passed();
                let text = scenario::report_text(&report);
                print!("{text}");
                util::write(&run_dir.join(format!("{}.report.txt", r.name)), &text)?;
                if let Some(md) = a.opt("markdown") {
                    let shown = Path::new(&reference_path).strip_prefix(repo).unwrap_or(Path::new(&reference_path));
                    util::write(Path::new(md), scenario::report_markdown(&report, &shown.to_string_lossy()))?;
                }
                reports.push(report);
            }
            if reports.is_empty() {
                return Err("the reference has no scenario runs".into());
            }
            util::write_json(&run_dir.join("summary.json"), &reports)?;
            println!("run directory: {}", run_dir.display());
            Ok(u8::from(!all_passed))
        }
        _ => Err(usage.into()),
    }
}
