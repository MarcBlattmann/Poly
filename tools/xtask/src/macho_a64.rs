//! arm64 Mach-O programs on x86-64 Poly through the translator (ADR 0257).
//!
//! The programs are the committed test files of `tests/a64/xnu/bin` (built
//! by its `build.sh` from our own C sources; the dynamic ones link a
//! stand-in libSystem of ours, never Apple's) and poly-dyld's `hello_a64`,
//! so the boot test needs no compiler. The image holds them, the stand-in
//! libSystem, libgreet, and the translator's runtime
//! (`/usr/libexec/poly-a64`, from `userland/native/a64`). Each runs as the
//! first process and must print its lines and exit with status 0:
//!
//! - `/bin/a64-hello-static`: static (`LC_UNIXTHREAD`), raw `write` and
//!   `exit`, the carry flag and EBADF of a failed call;
//! - `/bin/a64-hello-fat`: the same in a universal file with only an
//!   arm64 slice;
//! - `/bin/a64-hello`: dynamically linked through Poly's dyld (initializers,
//!   thread-local variables, `malloc` on the runtime's `mmap`);
//! - `/bin/mhello-a64`: poly-dyld's arm64 fixture (`@rpath` libgreet,
//!   thread-local variables in two images);
//! - `/bin/a64-crash`: reads address 8; the runtime stops it and the
//!   process ends with SIGSEGV through `kill`;
//! - `/bin/a64-heap 2`: malloc-heavy, under the JIT and then under the
//!   interpreter alone; both must print what the program printed natively,
//!   and the runtime's statistics give the speed of each.

use crate::initramfs::{self, Tree};
use crate::symbols::Symbols;
use crate::{Boot, DEFAULT_MEMORY, Devices, QEMU_SUCCESS, Result, boot_on, native, root};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

const BIN: &str = "tests/a64/xnu/bin";
const DYLD: &str = "crates/poly-dyld/tests/data";

const STATIC_LINES: &[&str] = &[
    "a64-static: hello from an arm64 program",
    "a64-static: argc=2 argv[0]=/bin/a64-hello-static",
    "a64-static: write to fd 99: carry set, EBADF",
    "a64-static: write returns its count",
    "a64-static: done",
];

const HELLO_LINES: &[&str] = &[
    "a64-hello: hello from arm64 through Poly's dyld",
    "ok initializers ran, libSystem's first",
    "ok __mh_execute_header is the mapped header",
    "ok thread-local variables",
    "ok malloc gives writable memory",
    "ok argc and argv",
    "ok envp ends with null",
    "ok apple[0] is executable_path=",
    "a64-hello: done",
];

const MHELLO_LINES: &[&str] = &[
    "libgreet: init",
    "hello: init",
    "ok initializers ran, the library's first",
    "greet: hello, mach-o",
    "ok greet binds across images",
    "ok thread-local variables",
    "ok apple[0] is executable_path=",
    "hello: done",
];

fn read(rel: &str) -> Result<Vec<u8>> {
    let p = root().join(rel);
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Check that `lines` appear in `output` in order.
pub(crate) fn in_order(program: &str, output: &str, lines: &[String]) -> Result {
    let mut at = 0;
    for line in lines {
        match output[at..].find(line.as_str()) {
            Some(i) => at += i + line.len(),
            None => return Err(format!("{program}: {line:?} missing or out of order")),
        }
    }
    Ok(())
}

/// One boot running `/bin/<program> <args>` with `env` as the first
/// process; it must exit with status 0.
fn run(kernel: &Path, syms: &Arc<Symbols>, image: &Path, program: &str, args: &[&str], env: &[&str]) -> Result<Boot> {
    run_ending(kernel, syms, image, program, args, env, "code=0")
}

/// [`run`] for a program that ends as `end` says (`code=<n>` or
/// `signal=<n>` of its `POLY-EXIT` record).
pub(crate) fn run_ending(
    kernel: &Path,
    syms: &Arc<Symbols>,
    image: &Path,
    program: &str,
    args: &[&str],
    env: &[&str],
    end: &str,
) -> Result<Boot> {
    let mut append = format!("poly.qemu_exit poly.init=/bin/{program}");
    for a in args {
        append.push_str(&format!(" poly.init.arg={a}"));
    }
    for e in env {
        append.push_str(&format!(" poly.init.env={e}"));
    }
    let t = Instant::now();
    let b = boot_on(None, Some(image), kernel, syms, DEFAULT_MEMORY, &append, Devices::None)?;
    let started = b.output.contains(&format!("POLY-INIT: exec /bin/{program}"));
    let record = format!("POLY-EXIT: pid=1 {end}");
    let exited = b.output.lines().any(|l| l.trim_end().ends_with(&record));
    if b.status != Some(QEMU_SUCCESS) || !started || !exited {
        let said: Vec<&str> = b.output.lines().filter(|l| l.contains("poly-a64") || l.contains("init:")).collect();
        return Err(format!(
            "/bin/{program}: started {started}, ended with {end} {exited}, QEMU status {:?} ({})",
            b.status,
            said.join("; ")
        ));
    }
    if let Some(line) = b.output.lines().find(|l| l.contains("not ok ") || l.contains(" not implemented")) {
        return Err(format!("/bin/{program}: {line}"));
    }
    eprintln!("    /bin/{program}: {:.1} s boot to power-off", t.elapsed().as_secs_f64());
    Ok(b)
}

/// The runtime's statistics line: (the line, its cycle count).
fn stats(program: &str, b: &Boot) -> Result<(String, u64)> {
    let line = b
        .output
        .lines()
        .find(|l| l.contains("poly-a64: ") && l.trim_end().ends_with(" cycles"))
        .ok_or_else(|| format!("/bin/{program}: no statistics line from the runtime"))?;
    let line = line[line.find("poly-a64: ").unwrap_or(0)..].trim_end().to_string();
    let cycles = line
        .rsplit(", ")
        .next()
        .and_then(|w| w.strip_suffix(" cycles"))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("/bin/{program}: {line:?} has no cycle count"))?;
    Ok((line, cycles))
}

/// The boot test group `macho-a64`.
pub fn boot_test(kernel: &Path, syms: &Arc<Symbols>) -> Result {
    let files = [
        ("a64-hello-static", read(&format!("{BIN}/hello-static"))?),
        ("a64-hello-fat", read(&format!("{BIN}/hello-fat"))?),
        ("a64-hello", read(&format!("{BIN}/hello"))?),
        ("a64-heap", read(&format!("{BIN}/heap"))?),
        ("a64-crash", read(&format!("{BIN}/crash"))?),
        ("mhello-a64", read(&format!("{DYLD}/hello_a64"))?),
    ];
    let programs: Vec<(&str, &[u8])> = files.iter().map(|(n, f)| (*n, f.as_slice())).collect();
    let mut tree = Tree { files: native::runtime_files()?, ..Default::default() };
    for (at, file) in [
        ("usr/lib/libSystem.B.dylib", format!("{BIN}/libSystem.B.dylib")),
        ("usr/lib/poly-dyld-test/libgreet.dylib", format!("{DYLD}/libgreet_a64.dylib")),
    ] {
        tree.files.push((at.into(), 0o755, read(&file)?));
    }
    let image =
        initramfs::write(&root(), "poly-macho-a64-initramfs.cpio", &initramfs::programs_image_with(&programs, &tree))?;

    for program in ["a64-hello-static", "a64-hello-fat"] {
        eprintln!(
            "==> boot test: a static arm64 Mach-O program runs on x86-64 Poly through the translator ({program})"
        );
        let b = run(kernel, syms, &image, program, &["x"], &[])?;
        let lines: Vec<String> =
            STATIC_LINES.iter().map(|l| l.replace("/bin/a64-hello-static", &format!("/bin/{program}"))).collect();
        in_order(program, &b.output, &lines)?;
    }

    eprintln!("==> boot test: a dynamically linked arm64 Mach-O program through Poly's dyld and the translator");
    let b = run(kernel, syms, &image, "a64-hello", &[], &[])?;
    in_order("a64-hello", &b.output, &HELLO_LINES.iter().map(|l| l.to_string()).collect::<Vec<_>>())?;

    eprintln!("==> boot test: poly-dyld's arm64 fixture (@rpath dylib, thread-local variables in two images)");
    let b = run(kernel, syms, &image, "mhello-a64", &[], &[])?;
    in_order("mhello-a64", &b.output, &MHELLO_LINES.iter().map(|l| l.to_string()).collect::<Vec<_>>())?;

    eprintln!("==> boot test: an arm64 program that reads address 8 ends with SIGSEGV");
    let b = run_ending(kernel, syms, &image, "a64-crash", &[], &[], "signal=11")?;
    let lines = ["a64-crash: about to read address 8", "poly-a64: /bin/a64-crash: stopped at pc"].map(String::from);
    in_order("a64-crash", &b.output, &lines)?;
    // Ended by kill(getpid(), SIGSEGV), not by the runtime's fallback fault.
    if let Some(line) = b.output.lines().find(|l| l.contains("user: pid 1 thread")) {
        return Err(format!("/bin/a64-crash: the runtime faulted instead of being killed: {line}"));
    }

    let want = String::from_utf8(read(&format!("{BIN}/heap-2.out"))?).map_err(|e| e.to_string())?;
    let want: Vec<String> = want.lines().map(String::from).collect();
    let mut cycles = Vec::new();
    for (engine, env) in [("JIT", "POLY_A64_ENGINE=jit"), ("interpreter", "POLY_A64_ENGINE=interp")] {
        eprintln!("==> boot test: a malloc-heavy arm64 program under the {engine}");
        let b = run(kernel, syms, &image, "a64-heap", &["2"], &[env, "POLY_A64_STATS=1"])?;
        in_order("a64-heap", &b.output, &want)?;
        let (line, n) = stats("a64-heap", &b)?;
        eprintln!("    {line}");
        cycles.push(n);
    }
    if let [jit, interp] = cycles[..] {
        eprintln!(
            "    a64-heap: the JIT took {:.2}x fewer cycles than the interpreter",
            interp as f64 / jit.max(1) as f64
        );
    }
    Ok(())
}
