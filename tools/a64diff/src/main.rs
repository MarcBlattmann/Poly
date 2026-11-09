//! `a64diff`: a differential tester for `poly-a64` (ADR 0152). Host tool, not
//! shipped, Linux only by nature.
//!
//! It builds static arm64 Linux test programs, runs each under the system's
//! `qemu-aarch64` (user mode) and under poly-a64, and prints pass or fail and
//! the register differences. `qemu-aarch64`, `clang` and `lld` are used as
//! black boxes (CLEAN_ROOM.md): their output is compared, their source is
//! never read.
//!
//! ```text
//! a64diff list                      list the hand-written programs and fuzz families
//! a64diff run [NAME...]             tests/a64/*.s: build, run both, compare output
//! a64diff fuzz [--seed N] [--blocks N] [--family NAME] [--keep DIR]
//!                                   generated one-instruction blocks with random state
//! ```
//!
//! On a host that is not Linux it prints that and exits successfully, so it
//! builds (and `cargo xtask ci lint` checks it) on Windows too.
//!
//! Environment: `A64_CC` (default `clang`), `A64_QEMU` (default
//! `qemu-aarch64`), `A64_QEMU_CPU` (default `max,pauth=off`: pointer
//! authentication is not implemented by poly-a64, and qemu's keys are
//! random).

mod elf;
mod fuzz;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use fuzz::{Case, FAMILIES, REC_BYTES, REC_FPSR, REC_NZCV, REC_SP, REC_V0, REC_WIN, REC_WORDS, REC_X0, Rng, State};

struct Tools {
    cc: String,
    qemu: String,
    qemu_cpu: String,
    work: PathBuf,
    include: PathBuf,
}

impl Tools {
    fn from_env() -> Tools {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let work = root.join("target").join("a64diff");
        let _ = std::fs::create_dir_all(&work);
        Tools {
            cc: std::env::var("A64_CC").unwrap_or_else(|_| "clang".into()),
            qemu: std::env::var("A64_QEMU").unwrap_or_else(|_| "qemu-aarch64".into()),
            qemu_cpu: std::env::var("A64_QEMU_CPU").unwrap_or_else(|_| "max,pauth=off".into()),
            work,
            include: root.join("tests").join("a64"),
        }
    }

    fn have(&self, tool: &str) -> bool {
        Command::new(tool).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
    }

    /// Assemble and link `asm` into a static executable.
    fn build(&self, asm: &Path, exe: &Path) -> Result<(), String> {
        let out = Command::new(&self.cc)
            .args(["--target=aarch64-linux-gnu", "-march=armv8.4-a+lse", "-nostdlib", "-static", "-fuse-ld=lld"])
            .arg("-Wl,--build-id=none")
            .arg("-I")
            .arg(&self.include)
            .arg("-o")
            .arg(exe)
            .arg(asm)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.cc))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("{} failed:\n{}", self.cc, String::from_utf8_lossy(&out.stderr)))
        }
    }

    /// Run under qemu-aarch64: the standard output and whether it exited with status 0.
    fn qemu_run(&self, exe: &Path) -> Result<(Vec<u8>, bool), String> {
        let out = Command::new(&self.qemu)
            .args(["-cpu", &self.qemu_cpu])
            .arg(exe)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.qemu))?;
        Ok((out.stdout, out.status.success()))
    }

    /// What `DCZID_EL0` reads under qemu (it is implementation defined; poly-a64 is told the same).
    fn dczid(&self) -> Result<u64, String> {
        let asm = ".text\n.globl _start\n_start:\n mrs x0, dczid_el0\n adrp x1, buf\n add x1, x1, :lo12:buf\n str x0, [x1]\n \
                   mov x0, #1\n mov x2, #8\n mov x8, #64\n svc #0\n mov x0, #0\n mov x8, #93\n svc #0\n.bss\nbuf: .space 8\n";
        let (s, e) = (self.work.join("dczid.s"), self.work.join("dczid"));
        std::fs::write(&s, asm).map_err(|e| e.to_string())?;
        self.build(&s, &e)?;
        let (out, ok) = self.qemu_run(&e)?;
        if !ok || out.len() != 8 {
            return Err("cannot read DCZID_EL0 under qemu".into());
        }
        Ok(u64::from_le_bytes(out.try_into().unwrap()))
    }

    /// Disassemble words with the system's `llvm-mc` (best effort, for messages).
    fn disasm(&self, words: &[u32]) -> Vec<String> {
        let mut bytes = String::new();
        for w in words {
            for b in w.to_le_bytes() {
                bytes.push_str(&format!("0x{b:02x} "));
            }
            bytes.push('\n');
        }
        let child = Command::new("llvm-mc")
            .args(["--disassemble", "--triple=aarch64", "-mattr=+v8.4a,+lse,+fp16,+neon"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return vec![] };
        {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(bytes.as_bytes());
            }
        }
        let Ok(out) = child.wait_with_output() else { return vec![] };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('.'))
            .collect()
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !cfg!(target_os = "linux") {
        println!("a64diff needs Linux (qemu-aarch64 user mode and a static arm64 linker); skipping on this host.");
        return ExitCode::SUCCESS;
    }
    let tools = Tools::from_env();
    match args.first().map(String::as_str) {
        Some("list") => {
            for p in programs(&tools) {
                println!("program  {}", p.file_stem().unwrap_or_default().to_string_lossy());
            }
            for f in FAMILIES {
                println!("family   {}", f.name);
            }
            ExitCode::SUCCESS
        }
        Some(cmd @ ("run" | "fuzz")) => {
            for t in [tools.cc.as_str(), tools.qemu.as_str()] {
                if !tools.have(t) {
                    eprintln!("a64diff: `{t}` not found (set A64_CC / A64_QEMU, or run in the poly-dev image)");
                    return ExitCode::from(2);
                }
            }
            let result = if cmd == "run" { run_programs(&tools, &args[1..]) } else { run_fuzz(&tools, &args[1..]) };
            match result {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("a64diff: {e}");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!(
                "usage: a64diff list | run [NAME...] | fuzz [--seed N] [--blocks N] [--family NAME] [--keep DIR]"
            );
            ExitCode::from(2)
        }
    }
}

fn programs(tools: &Tools) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(&tools.include)
        .map(|d| {
            d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "s")).collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

// ---------------------------------------------------------------------
// Hand-written programs
// ---------------------------------------------------------------------

/// The names of the 8-byte words of the dump written by `tests/a64/dump.inc`.
fn dump_word_name(i: usize) -> String {
    match i {
        0..=30 => format!("x{i}"),
        31 => "nzcv".into(),
        32 => "fpsr".into(),
        _ => format!("v{}.{}", (i - 33) / 2, if (i - 33).is_multiple_of(2) { "lo" } else { "hi" }),
    }
}

fn run_programs(tools: &Tools, names: &[String]) -> Result<bool, String> {
    let mut ok = true;
    let mut ran = 0;
    for p in programs(tools) {
        let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if !names.is_empty() && !names.contains(&stem) {
            continue;
        }
        ran += 1;
        let exe = tools.work.join(&stem);
        tools.build(&p, &exe)?;
        let (q_out, q_ok) = tools.qemu_run(&exe)?;
        let elf = std::fs::read(&exe).map_err(|e| e.to_string())?;
        let r = elf::run(&elf, 50_000_000, tools.dczid()?)?;
        let p_ok = r.end == elf::End::Exit(0);
        if q_ok && p_ok && q_out == r.stdout {
            println!("PASS {stem} ({} instructions)", r.steps);
            continue;
        }
        ok = false;
        println!("FAIL {stem}");
        if !q_ok {
            println!("  qemu did not exit with status 0");
        }
        if !p_ok {
            println!("  poly-a64: {:?}", r.end);
        }
        let words =
            |b: &[u8]| -> Vec<u64> { b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect() };
        let (qw, pw) = (words(&q_out), words(&r.stdout));
        if qw.len() != pw.len() {
            println!("  output size differs: qemu {} bytes, poly {} bytes", q_out.len(), r.stdout.len());
        }
        for i in 0..qw.len().min(pw.len()) {
            if qw[i] != pw[i] {
                println!("  {:8} qemu={:#018x} poly={:#018x}", dump_word_name(i), qw[i], pw[i]);
            }
        }
    }
    if ran == 0 {
        return Err("no program matched".into());
    }
    Ok(ok)
}

// ---------------------------------------------------------------------
// Fuzzing
// ---------------------------------------------------------------------

struct FuzzOpts {
    seed: u64,
    blocks: usize,
    family: Option<String>,
    keep: Option<PathBuf>,
}

fn parse_fuzz(args: &[String]) -> Result<FuzzOpts, String> {
    let mut o = FuzzOpts { seed: 1, blocks: 600, family: None, keep: None };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--seed" => o.seed = val()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--blocks" => o.blocks = val()?.parse().map_err(|e| format!("--blocks: {e}"))?,
            "--family" => o.family = Some(val()?),
            "--keep" => o.keep = Some(PathBuf::from(val()?)),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(o)
}

fn words_of(b: &[u8]) -> Vec<u64> {
    b.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Build and run on qemu a subset of cases; true if it ran to completion.
fn qemu_ok(tools: &Tools, cases: &[(Case, State)], tags: &[u32], idx: &[usize], n: usize) -> Result<bool, String> {
    let asm = subset_program(cases, tags, idx);
    let asm_path = tools.work.join(format!("bisect{n}.s"));
    let exe = tools.work.join(format!("bisect{n}"));
    std::fs::write(&asm_path, asm).map_err(|e| e.to_string())?;
    tools.build(&asm_path, &exe)?;
    Ok(tools.qemu_run(&exe)?.1)
}

fn subset_program(cases: &[(Case, State)], tags: &[u32], idx: &[usize]) -> String {
    // The generator numbers blocks by position, so build the subset with fresh positions and
    // keep the original index as the tag.
    let sub_cases: Vec<(Case, State)> = idx
        .iter()
        .map(|&i| {
            let (c, s) = &cases[i];
            (
                Case { family: c.family, setup: c.setup.clone(), body: c.body.clone() },
                State { x: s.x, sp: s.sp, nzcv: s.nzcv, fpcr: s.fpcr, v: s.v, window: s.window },
            )
        })
        .collect();
    let sub_tags: Vec<u32> = idx.iter().map(|&i| tags[i]).collect();
    fuzz::program(&sub_cases, &sub_tags)
}

/// Find the cases qemu itself rejects (a bad encoding in the generator).
fn find_rejects(
    tools: &Tools,
    cases: &[(Case, State)],
    tags: &[u32],
    idx: &[usize],
    counter: &mut usize,
    bad: &mut Vec<usize>,
) -> Result<(), String> {
    if idx.is_empty() {
        return Ok(());
    }
    *counter += 1;
    if qemu_ok(tools, cases, tags, idx, *counter)? {
        return Ok(());
    }
    if idx.len() == 1 {
        bad.push(idx[0]);
        return Ok(());
    }
    let (a, b) = idx.split_at(idx.len() / 2);
    find_rejects(tools, cases, tags, a, counter, bad)?;
    find_rejects(tools, cases, tags, b, counter, bad)
}

fn run_fuzz(tools: &Tools, args: &[String]) -> Result<bool, String> {
    let opts = parse_fuzz(args)?;
    let fams: Vec<&fuzz::Family> =
        FAMILIES.iter().filter(|f| opts.family.as_ref().is_none_or(|n| n == f.name)).collect();
    if fams.is_empty() {
        return Err("no such family (see `a64diff list`)".into());
    }
    let mut rng = Rng(opts.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut cases: Vec<(Case, State)> = Vec::new();
    for i in 0..opts.blocks {
        let f = fams[i % fams.len()];
        let c = (f.make)(&mut rng);
        cases.push((c, State::random(&mut rng)));
    }
    let tags: Vec<u32> = (0..cases.len() as u32).collect();
    let all: Vec<usize> = (0..cases.len()).collect();

    // 1. Drop what qemu rejects (our generator's mistakes), reporting each.
    let mut rejects = Vec::new();
    let mut counter = 0;
    find_rejects(tools, &cases, &tags, &all, &mut counter, &mut rejects)?;
    for &i in &rejects {
        println!("generator: qemu rejects [{}] {}", cases[i].0.family, cases[i].0.body.join(" ; "));
    }
    let valid: Vec<usize> = all.iter().copied().filter(|i| !rejects.contains(i)).collect();

    // 2. Build the final program and run both.
    let asm = subset_program(&cases, &tags, &valid);
    let asm_path = tools.work.join("fuzz.s");
    let exe = tools.work.join("fuzz");
    std::fs::write(&asm_path, &asm).map_err(|e| e.to_string())?;
    tools.build(&asm_path, &exe)?;
    let (q_out, q_ok) = tools.qemu_run(&exe)?;
    if !q_ok {
        return Err("qemu failed on the final program".into());
    }
    let elf_bytes = std::fs::read(&exe).map_err(|e| e.to_string())?;
    let (p_out, stops) = elf::run_blocks(&elf_bytes, 2000, tools.dczid()?)?;
    if let Some(dir) = &opts.keep {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::copy(&asm_path, dir.join("fuzz.s"));
        let _ = std::fs::copy(&exe, dir.join("fuzz"));
    }

    // 3. Compare record by record (matched by tag).
    let recs = |b: &[u8]| -> BTreeMap<u32, Vec<u64>> {
        let mut m = BTreeMap::new();
        for chunk in b.chunks_exact(REC_BYTES) {
            let w = words_of(chunk);
            m.insert(w[0] as u32, w);
        }
        m
    };
    let (qr, pr) = (recs(&q_out), recs(&p_out));
    let mut per_family: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new(); // pass, fail, stopped
    let mut reported = 0;
    let mut all_ok = true;
    for stop in &stops {
        // The tag of position `block` in the final program.
        let orig = valid[stop.block];
        let e = per_family.entry(cases[orig].0.family).or_default();
        e.2 += 1;
        if e.2 <= 3 {
            let w = word_of(&cases[orig].0);
            let d = w.map(|w| tools.disasm(&[w])).and_then(|v| v.into_iter().next()).unwrap_or_default();
            println!(
                "poly-a64 stopped [{}] {} {}: {}",
                cases[orig].0.family,
                cases[orig].0.body.join(" ; "),
                d,
                stop.why
            );
        }
    }
    for (pos, &orig) in valid.iter().enumerate() {
        let fam = cases[orig].0.family;
        let entry = per_family.entry(fam).or_default();
        if stops.iter().any(|s| s.block == pos) {
            continue;
        }
        let (Some(q), Some(p)) = (qr.get(&(orig as u32)), pr.get(&(orig as u32))) else {
            entry.1 += 1;
            all_ok = false;
            continue;
        };
        let diffs = diff_record(q, p);
        if diffs.is_empty() {
            entry.0 += 1;
        } else {
            entry.1 += 1;
            all_ok = false;
            if reported < 25 {
                reported += 1;
                let w = word_of(&cases[orig].0);
                let d = w.map(|w| tools.disasm(&[w])).and_then(|v| v.into_iter().next()).unwrap_or_default();
                println!("MISMATCH [{fam}] {}   {d}", cases[orig].0.body.join(" ; "));
                if !cases[orig].0.setup.is_empty() {
                    println!("  setup: {}", cases[orig].0.setup.join(" ; "));
                }
                for l in diffs.iter().take(12) {
                    println!("  {l}");
                }
                if diffs.len() > 12 {
                    println!("  ... and {} more", diffs.len() - 12);
                }
            }
        }
    }
    println!();
    println!("{:<18} {:>6} {:>6} {:>8}", "family", "pass", "fail", "stopped");
    let (mut tp, mut tf, mut ts) = (0, 0, 0);
    for (fam, (p, f, s)) in &per_family {
        println!("{fam:<18} {p:>6} {f:>6} {s:>8}");
        tp += p;
        tf += f;
        ts += s;
    }
    println!("{:<18} {:>6} {:>6} {:>8}  ({} rejected by qemu, seed {})", "total", tp, tf, ts, rejects.len(), opts.seed);
    Ok(all_ok && stops.is_empty())
}

/// The single instruction word of a case, if its body is one `.inst` line.
fn word_of(c: &Case) -> Option<u32> {
    let l = c.body.first()?;
    let h = l.strip_prefix(".inst 0x")?;
    u32::from_str_radix(h, 16).ok()
}

fn diff_record(q: &[u64], p: &[u64]) -> Vec<String> {
    let mut out = Vec::new();
    assert_eq!(q.len(), REC_WORDS);
    for i in 0..31 {
        if q[REC_X0 + i] != p[REC_X0 + i] {
            out.push(format!("x{i:<2}  qemu={:#018x} poly={:#018x}", q[REC_X0 + i], p[REC_X0 + i]));
        }
    }
    if q[REC_SP] != p[REC_SP] {
        out.push(format!("sp   qemu={:#018x} poly={:#018x}", q[REC_SP], p[REC_SP]));
    }
    if q[REC_NZCV] != p[REC_NZCV] {
        out.push(format!("nzcv qemu={:#010x} poly={:#010x}", q[REC_NZCV], p[REC_NZCV]));
    }
    // Only the exception flags poly-a64 models (IOC, DZC).
    if (q[REC_FPSR] ^ p[REC_FPSR]) & 3 != 0 {
        out.push(format!("fpsr qemu={:#x} poly={:#x} (IOC/DZC bits)", q[REC_FPSR] & 3, p[REC_FPSR] & 3));
    }
    for i in 0..32 {
        let (ql, qh) = (q[REC_V0 + 2 * i], q[REC_V0 + 2 * i + 1]);
        let (pl, ph) = (p[REC_V0 + 2 * i], p[REC_V0 + 2 * i + 1]);
        if ql != pl || qh != ph {
            out.push(format!("v{i:<2}  qemu={qh:016x}_{ql:016x} poly={ph:016x}_{pl:016x}"));
        }
    }
    for i in 0..64 {
        if q[REC_WIN + i] != p[REC_WIN + i] {
            out.push(format!(
                "mem[x5{:+}] qemu={:#018x} poly={:#018x}",
                8 * i as i64 - 256,
                q[REC_WIN + i],
                p[REC_WIN + i]
            ));
        }
    }
    out
}
