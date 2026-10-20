//! Dynamically linked programs for the boot tests of roadmap Phase 4,
//! "glibc's own `ld.so` loads shared libraries unchanged" (ADR 0049).
//!
//! The programs (`tools/xtask/programs/dynamic/`) are built at test time
//! with the host's C (and C++) compiler, without `-static`:
//!
//! - `/bin/dyn-hello`: stdio, the heap, the environment, `getauxval(3)`;
//!   `/bin/dyn-hello-nopie` is the same linked as a position-dependent
//!   executable (`ET_EXEC` with `PT_INTERP`);
//! - `/bin/dyn-dlopen`: `dlopen(3)` of `/lib/poly/libpolyplugin.so`
//!   (`plugin.c`, found through `DT_RUNPATH` `$ORIGIN/../lib/poly`),
//!   which needs libm itself;
//! - `/bin/dyn-threads`: POSIX threads, TLS, mutexes, condition variables;
//! - `/bin/dyn-cxx`: the C++ standard library and exceptions, if the host
//!   has a C++ compiler.
//!
//! The image also gets the files the dynamic linker will look for: the
//! interpreter each program names in `PT_INTERP`, and every shared object
//! in the `DT_NEEDED` closure of the programs and the plugin, copied from
//! the host (`libsearch.rs`, shared with the pipeline's app tests). They
//! are found the way ld.so(8) describes the search (the object's
//! `DT_RPATH` or `DT_RUNPATH`, the directories of `/etc/ld.so.conf`, then
//! `/lib64`, `/usr/lib64`, `/lib`, `/usr/lib`),
//! reading only the files' ELF headers (`poly_elf::Dynamic`); nothing is
//! run on the host. Each library goes to the path it was found at, with
//! links to it from `/lib`, `/usr/lib`, `/lib64` and `/usr/lib64`, so the
//! dynamic linker finds it in its default directories without
//! `/etc/ld.so.cache` (which the image does not have).
//!
//! Nothing here is committed: the C library is the host's, only copied
//! into the test initramfs. Without a C compiler or a shared C library the
//! tests are skipped with the reason.
//!
//! The boot tests ([`boot_tests`]) run every program as the first process
//! (`poly.init`), then all of them from the shell's `run`, the dynamic
//! linker as a program (`run /lib64/ld-linux-x86-64.so.2 /bin/dyn-hello`)
//! and `run LD_DEBUG=libs /bin/dyn-hello`; each must print its line and
//! exit with status 0, and no system call but `rseq` may be missing.
//! `cargo xtask dynamic-initramfs` writes the image for a boot by hand.

use crate::initramfs::Tree;
use crate::libsearch;
use poly_elf::Elf;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the sources are, relative to the repository.
const SOURCES: &str = "tools/xtask/programs/dynamic";

/// The plugin `dyn-dlopen` loads, in the image.
const PLUGIN_PATH: &str = "lib/poly/libpolyplugin.so";

/// A program of the image: its name in `/bin` and the line it prints when
/// every check passed.
#[derive(Clone, Copy, Debug)]
pub struct Program {
    pub name: &'static str,
    pub output: &'static str,
}

pub const HELLO: Program = Program { name: "dyn-hello", output: "dyn-hello: hello, world\n" };
pub const HELLO_NOPIE: Program = Program { name: "dyn-hello-nopie", output: "dyn-hello: hello, world\n" };
pub const DLOPEN: Program = Program { name: "dyn-dlopen", output: "dyn-dlopen: ok\n" };
pub const THREADS: Program = Program { name: "dyn-threads", output: "dyn-threads: ok\n" };
pub const CXX: Program = Program { name: "dyn-cxx", output: "dyn-cxx: ok\n" };

/// The built programs and the files they need.
#[derive(Debug, Default)]
pub struct Built {
    /// `(program, ELF)` for `/bin`.
    pub programs: Vec<(Program, Vec<u8>)>,
    /// The interpreter, the libraries, the plugin and the links to them.
    pub tree: Tree,
    /// The interpreter's path, as the programs name it in `PT_INTERP`.
    pub interp: String,
    /// What was left out and why (no C++ compiler, ...).
    pub notes: Vec<String>,
}

fn out_dir() -> PathBuf {
    super::root().join("target/xtask-programs/dynamic")
}

/// Compile `sources` with `compiler` and `flags` (given after the sources,
/// so libraries follow what needs them) into
/// `target/xtask-programs/dynamic/<name>`.
fn compile(compiler: &str, sources: &[&str], flags: &[&str], name: &str) -> Result<Vec<u8>, String> {
    let out = out_dir().join(name);
    let mut cmd = Command::new(compiler);
    cmd.current_dir(super::root());
    cmd.args(["-O2", "-g0", "-Wall", "-Wl,--build-id=none", "-o"]).arg(&out);
    cmd.args(sources.iter().map(|s| format!("{SOURCES}/{s}"))).args(flags);
    let result = cmd.output().map_err(|e| format!("{compiler}: {e}"))?;
    if !result.status.success() {
        return Err(format!("{compiler} {}: {}", sources.join(" "), String::from_utf8_lossy(&result.stderr).trim()));
    }
    std::fs::read(&out).map_err(|e| format!("{}: {e}", out.display()))
}

/// Build the programs and collect what they need; `Err` with the reason
/// when the host cannot build dynamically linked C programs.
pub fn build() -> Result<Built, String> {
    std::fs::create_dir_all(out_dir()).map_err(|e| format!("{}: {e}", out_dir().display()))?;
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut built = Built::default();
    let c_programs: [(Program, &[&str], &[&str]); 4] = [
        (HELLO, &["hello.c"], &[]),
        (HELLO_NOPIE, &["hello.c"], &["-no-pie", "-fno-pie"]),
        (DLOPEN, &["dlopen.c"], &["-Wl,-rpath,$ORIGIN/../lib/poly", "-ldl", "-lm"]),
        (THREADS, &["threads.c"], &["-pthread"]),
    ];
    for (program, sources, flags) in c_programs {
        let elf = compile(&cc, sources, flags, program.name)?;
        built.programs.push((program, elf));
    }
    let plugin = compile(&cc, &["plugin.c"], &["-shared", "-fPIC", "-lm"], "libpolyplugin.so")?;
    let cxx = std::env::var("CXX").unwrap_or_else(|_| "g++".into());
    match compile(&cxx, &["cxx.cpp"], &["-std=c++17"], CXX.name) {
        Ok(elf) => built.programs.push((CXX, elf)),
        Err(e) => built.notes.push(format!("no C++ program: {e}")),
    }

    // Every program must name the same interpreter, which must exist.
    let mut interp = None;
    for (program, elf) in &built.programs {
        let parsed = Elf::parse(elf).map_err(|e| format!("{}: {e:?}", program.name))?;
        let Some(path) = parsed.interp() else {
            return Err(format!("{} was linked statically: the host has no shared C library", program.name));
        };
        let path = String::from_utf8_lossy(path).into_owned();
        if interp.get_or_insert_with(|| path.clone()) != &path {
            return Err(format!("the programs name two interpreters ({path} and {})", interp.unwrap_or_default()));
        }
    }
    let interp = interp.ok_or("no programs")?;
    let interp_bytes = std::fs::read(&interp).map_err(|e| format!("the interpreter {interp}: {e}"))?;
    Elf::parse(&interp_bytes).map_err(|e| format!("the interpreter {interp} is not an x86-64 ELF file: {e:?}"))?;
    let interp_name = file_name(&interp);
    built.tree.files.push((interp.trim_start_matches('/').to_string(), 0o755, interp_bytes));

    // The DT_NEEDED closure, breadth first, as the dynamic linker loads it.
    let mut objects: Vec<(String, Vec<u8>)> =
        built.programs.iter().map(|(program, elf)| (program.name.to_string(), elf.clone())).collect();
    objects.push(("libpolyplugin.so".into(), plugin.clone()));
    let found = libsearch::needed_closure(&objects, &interp_name)?;
    if !found.contains_key("libc.so.6") {
        return Err("the programs do not need libc.so.6: not a glibc host".into());
    }
    for (name, path) in &found {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let at = path.parent().unwrap_or(Path::new("/")).join(name);
        let at = at.to_string_lossy().trim_start_matches('/').to_string();
        built.tree.files.push((at.clone(), 0o755, bytes));
        default_links(&mut built.tree, name, &at);
    }
    // libc names the interpreter by its own name: a link where libraries are.
    if let Some(libc) = found.get("libc.so.6") {
        let at = libc.parent().unwrap_or(Path::new("/")).join(&interp_name);
        let at = at.to_string_lossy().trim_start_matches('/').to_string();
        if at != interp.trim_start_matches('/') {
            built.tree.symlinks.push((at, interp.clone()));
        }
    }
    built.tree.files.push((PLUGIN_PATH.into(), 0o755, plugin));
    built.interp = interp;
    Ok(built)
}

/// Links named `name` to `/<at>` in the default directories ld.so(8)
/// lists, where `at` is not already there.
fn default_links(tree: &mut Tree, name: &str, at: &str) {
    for dir in ["lib", "usr/lib", "lib64", "usr/lib64"] {
        let link = format!("{dir}/{name}");
        let taken = tree.files.iter().any(|(p, _, _)| *p == link) || tree.symlinks.iter().any(|(p, _)| *p == link);
        if link != at && !taken {
            tree.symlinks.push((link, format!("/{at}")));
        }
    }
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Build the programs and write the image `target/poly-dynamic-initramfs.cpio`
/// (the default image, the nolibc hello as `/bin/hello`, the dynamic
/// programs in `/bin` and what they need). `Ok(None)`, with the reason
/// printed, when the host cannot build them.
pub fn write_image() -> Result<Option<(PathBuf, Built)>, String> {
    let built = match build() {
        Ok(b) => b,
        Err(why) => {
            eprintln!("==> skipped: dynamically linked programs: {why}");
            return Ok(None);
        }
    };
    for note in &built.notes {
        eprintln!("==> note: {note}");
    }
    let hello = super::programs::nolibc_hello()?;
    let mut programs: Vec<(&str, &[u8])> = vec![("hello", &hello)];
    programs.extend(built.programs.iter().map(|(p, elf)| (p.name, elf.as_slice())));
    let image = crate::initramfs::programs_image_with(&programs, &built.tree);
    let path = crate::initramfs::write(&super::root(), "poly-dynamic-initramfs.cpio", &image)?;
    Ok(Some((path, built)))
}

/// `cargo xtask dynamic-initramfs`: [`write_image`] and say where it went.
pub fn command() -> super::Result {
    match write_image()? {
        Some((path, built)) => {
            let names: Vec<_> = built.programs.iter().map(|(p, _)| format!("/bin/{}", p.name)).collect();
            println!("{}", path.display());
            eprintln!("==> {} with {} (interpreter {})", path.display(), names.join(", "), built.interp);
            Ok(())
        }
        None => Err("the host cannot build dynamically linked programs".into()),
    }
}

/// System calls a dynamically linked glibc program may make that Poly
/// answers with `ENOSYS` on purpose: `rseq` (ADR 0043, 0049).
const EXPECTED_ENOSYS: [&str; 1] = [" rseq not implemented"];

/// The first "not implemented" line of a boot's log other than the
/// expected ones.
fn unimplemented(output: &str) -> Option<&str> {
    output.lines().find(|l| l.contains(" not implemented") && !EXPECTED_ENOSYS.iter().any(|e| l.contains(e)))
}

/// The boot tests: every program as the first process, then from the
/// shell (with the interpreter run as a program too).
pub fn boot_tests(kernel: &Path, syms: &std::sync::Arc<crate::symbols::Symbols>) -> super::Result {
    let Some((image, built)) = write_image()? else { return Ok(()) };
    for (program, _) in &built.programs {
        eprintln!(
            "==> boot test: {} (dynamically linked, through {}) runs as the first process",
            program.name, built.interp
        );
        let b = super::programs::run_as_init(kernel, syms, &image, program.name, program.output)?;
        if let Some(line) = unimplemented(&b.output) {
            return Err(format!("/bin/{} made a system call Poly does not implement: {line}", program.name));
        }
    }

    eprintln!(
        "==> boot test: the shell runs dynamically linked programs, the dynamic linker as a program, and LD_DEBUG"
    );
    // (command line, the program `run` reports, its output, more it must print)
    let mut runs: Vec<(String, String, &str, &str)> = built
        .programs
        .iter()
        .map(|(p, _)| (format!("run /bin/{}\r", p.name), format!("/bin/{}", p.name), p.output, ""))
        .collect();
    runs.push((format!("run {} /bin/{}\r", built.interp, HELLO.name), built.interp.clone(), HELLO.output, ""));
    // ld.so(8): LD_DEBUG=libs shows the library search on stderr.
    let hello = format!("/bin/{}", HELLO.name);
    runs.push((format!("run LD_DEBUG=libs {hello}\r"), hello, HELLO.output, "libc.so.6"));
    let mut commands: Vec<&str> = runs.iter().map(|(c, ..)| c.as_str()).collect();
    commands.push("reboot\r");
    let append = format!("poly.qemu_exit {} poly.test=user::exec_tests::", super::SHELL_FLAG);
    let qemu_args = ["-initrd".to_string(), image.display().to_string()];
    let steps: Vec<_> = commands.iter().map(|&c| (super::console::PROMPT, c)).collect();
    let b = super::boot_typing(kernel, syms, &append, super::Devices::None, &qemu_args, &steps)?;
    let ex = super::console::exchanges(&b.output);
    if ex.len() < commands.len() {
        return Err(format!(
            "the shell answered {} of {} command lines (status {:?})",
            ex.len(),
            commands.len(),
            b.status
        ));
    }
    for (i, (command, program, output, more)) in runs.iter().enumerate() {
        let out = ex[i].output;
        if !out.contains(output.trim_end())
            || !out.contains(more)
            || !out.ends_with(&format!("{program}: exit status 0\n"))
        {
            return Err(format!("the shell's {:?} did not print {:?} and exit 0: {out:?}", command.trim_end(), output));
        }
    }
    if let Some(line) = unimplemented(&b.output) {
        return Err(format!("a program run from the shell made a system call Poly does not implement: {line}"));
    }
    if b.status != Some(0) {
        return Err(format!("reboot did not reset the machine (status {:?})", b.status));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_links_skip_taken_paths() {
        let mut tree = Tree::default();
        tree.files.push(("lib64/ld.so".into(), 0o755, vec![]));
        default_links(&mut tree, "libc.so.6", "usr/lib/libc.so.6");
        default_links(&mut tree, "ld.so", "lib/x/ld.so");
        let links: Vec<_> = tree.symlinks.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
        assert_eq!(
            links,
            [
                ("lib/libc.so.6", "/usr/lib/libc.so.6"),
                ("lib64/libc.so.6", "/usr/lib/libc.so.6"),
                ("usr/lib64/libc.so.6", "/usr/lib/libc.so.6"),
                ("lib/ld.so", "/lib/x/ld.so"),
                ("usr/lib/ld.so", "/lib/x/ld.so"),
                ("usr/lib64/ld.so", "/lib/x/ld.so"),
            ]
        );
    }
}
