//! The Linux test corpus in `tests/linux/`: small static programs that
//! print machine-readable results (see `tests/linux/include/poly_test.h`).
//!
//! `tests/linux/<name>.c` are freestanding: no C library, raw syscalls, so
//! each makes exactly the syscalls it asks for. `tests/linux/libc/<name>.c`
//! link the host's static C library (named `libc-<name>`); their traces
//! show a C library's startup sequence. They are skipped where the host
//! has no static C library.
//!
//! Builds are reproducible: fixed flags, no build id, sources passed by
//! relative path, so the same compiler gives the same bytes. The manifest
//! records the compiler and the hash of every source and binary.

use crate::sha256;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const DIR: &str = "tests/linux";
pub const MANIFEST: &str = "manifest.json";

/// Static, no C library, no PIE, no stack protector or CET (`_start` is
/// ours), no unwind tables, no build id.
pub const FREESTANDING_FLAGS: &[&str] = &[
    "-static",
    "-nostdlib",
    "-ffreestanding",
    "-fno-builtin",
    "-fno-stack-protector",
    "-fcf-protection=none",
    "-fno-pie",
    "-no-pie",
    "-fno-asynchronous-unwind-tables",
    "-O2",
    "-Wall",
    "-Wextra",
    "-Werror",
    "-Wl,--build-id=none",
    "-Itests/linux/include",
];

pub const LIBC_FLAGS: &[&str] = &["-static", "-fno-pie", "-no-pie", "-O2", "-Wall", "-Werror", "-Wl,--build-id=none"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    /// Relative to the repository root.
    pub source: PathBuf,
    pub libc: bool,
}

fn c_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let Ok(read) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in read {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "c") && path.is_file() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Every program in the corpus, freestanding ones first, by name.
pub fn discover(root: &Path) -> Result<Vec<Program>> {
    let mut out = Vec::new();
    for (sub, libc) in [("", false), ("libc", true)] {
        for path in c_files(&root.join(DIR).join(sub))? {
            let stem = path.file_stem().unwrap().to_string_lossy();
            let name = if libc { format!("libc-{stem}") } else { stem.into_owned() };
            out.push(Program { name, source: path.strip_prefix(root).unwrap().to_path_buf(), libc });
        }
    }
    if out.is_empty() {
        return Err(format!("no test programs in {}", root.join(DIR).display()));
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Built {
    pub name: String,
    pub kind: String,
    pub source: String,
    pub source_sha256: String,
    /// File name in the output directory.
    pub binary: String,
    pub binary_sha256: String,
    /// The test's own syscall comparison mode (`poly-diff: syscalls=MODE`
    /// in its source), for programs whose syscall sequence depends on
    /// scheduling; see [`syscalls_directive`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syscalls: Option<String>,
}

/// The directive a test program may carry in its source, on a line of its
/// own (usually a comment): `poly-diff: syscalls=off` (or `names`). Tests
/// with more than one task interleave their syscalls, retry calls or get
/// results that depend on scheduling, so their syscall sequence cannot be
/// compared; their exit status and output still are. The mode only ever
/// weakens the run's (`--syscalls`), never strengthens it.
pub fn syscalls_directive(source: &str) -> Result<Option<String>> {
    for line in source.lines() {
        let Some((_, rest)) = line.split_once("poly-diff:") else { continue };
        let word = rest.split_whitespace().next().unwrap_or_default();
        return match word.strip_prefix("syscalls=") {
            Some(mode @ ("exact" | "names" | "off")) => Ok(Some(mode.to_string())),
            _ => Err(format!("poly-diff directive {:?}: expected syscalls=exact|names|off", line.trim())),
        };
    }
    Ok(None)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub cc: String,
    /// First line of `cc --version`.
    pub compiler: String,
    pub freestanding_flags: Vec<String>,
    pub libc_flags: Vec<String>,
    pub tests: Vec<Built>,
    pub skipped: Vec<Skipped>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Manifest> {
        util::read_json(&dir.join(MANIFEST))
    }

    pub fn find(&self, name: &str) -> Option<&Built> {
        self.tests.iter().find(|b| b.name == name)
    }
}

/// The C compiler: `--cc`, else `$CC`, else `cc`.
pub fn compiler(cc: Option<&str>) -> String {
    cc.map(str::to_string).or_else(|| std::env::var("CC").ok()).unwrap_or_else(|| "cc".into())
}

/// First line of `cc --version`, or an error if there is no compiler.
pub fn compiler_version(cc: &str) -> Result<String> {
    let out = Command::new(cc)
        .arg("--version")
        .output()
        .map_err(|e| format!("C compiler {cc:?}: {e} (install gcc or clang, or set CC)"))?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or_default().to_string())
}

/// Freestanding test programs in `dir` (an entry's visible or held-out
/// tests, written in the corpus format), named after their files.
pub fn discover_extra(root: &Path, dir: &Path) -> Result<Vec<Program>> {
    let files = c_files(dir)?;
    if files.is_empty() {
        return Err(format!("no .c test programs in {}", dir.display()));
    }
    Ok(files
        .into_iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let source = path.strip_prefix(root).map(Path::to_path_buf).unwrap_or(path);
            Program { name, source, libc: false }
        })
        .collect())
}

/// Compile every program in the corpus into `out`.
pub fn build(root: &Path, out: &Path, cc: &str) -> Result<Manifest> {
    build_with(root, out, cc, &[])
}

/// Compile the corpus and the test programs in `extra` directories.
pub fn build_with(root: &Path, out: &Path, cc: &str, extra: &[PathBuf]) -> Result<Manifest> {
    let mut programs = discover(root)?;
    for dir in extra {
        for p in discover_extra(root, &util::absolute(dir))? {
            if programs.iter().any(|q| q.name == p.name) {
                return Err(format!("two test programs are called {} ({})", p.name, p.source.display()));
            }
            programs.push(p);
        }
    }
    let compiler = compiler_version(cc)?;
    util::create_dir(out)?;
    let out = util::absolute(out);
    let mut manifest = Manifest {
        format: "poly-corpus/1".into(),
        cc: cc.into(),
        compiler,
        freestanding_flags: FREESTANDING_FLAGS.iter().map(|s| s.to_string()).collect(),
        libc_flags: LIBC_FLAGS.iter().map(|s| s.to_string()).collect(),
        tests: Vec::new(),
        skipped: Vec::new(),
    };
    for p in programs {
        let binary = out.join(&p.name);
        let flags = if p.libc { LIBC_FLAGS } else { FREESTANDING_FLAGS };
        let result = Command::new(cc)
            .current_dir(root)
            .args(flags)
            .arg(format!("-frandom-seed={}", p.name))
            .arg("-o")
            .arg(&binary)
            .arg(&p.source)
            .output()
            .map_err(|e| format!("{cc}: {e}"))?;
        if !result.status.success() {
            let msg = String::from_utf8_lossy(&result.stderr).trim().to_string();
            if p.libc {
                // No static C library here: not an error, the corpus just
                // lacks the C library startup trace.
                let reason = msg.lines().last().unwrap_or("link failed").to_string();
                manifest.skipped.push(Skipped { name: p.name, reason });
                continue;
            }
            return Err(format!("{}: compile failed:\n{msg}", p.source.display()));
        }
        let source = util::read(&root.join(&p.source))?;
        let syscalls = syscalls_directive(&String::from_utf8_lossy(&source))
            .map_err(|e| format!("{}: {e}", p.source.display()))?;
        manifest.tests.push(Built {
            name: p.name.clone(),
            kind: if p.libc { "libc" } else { "freestanding" }.into(),
            source: p.source.to_string_lossy().into_owned(),
            source_sha256: sha256::hex(&source),
            binary: p.name,
            binary_sha256: sha256::hex(&util::read(&binary)?),
            syscalls,
        });
    }
    util::write_json(&out.join(MANIFEST), &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        crate::repo_root()
    }

    #[test]
    fn discovers_the_corpus() {
        let programs = discover(&root()).unwrap();
        let names: Vec<&str> = programs.iter().map(|p| p.name.as_str()).collect();
        for want in ["hello", "stderr", "exit42", "brk", "mmap", "efault", "segv", "libc-hello"] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        assert!(programs.iter().find(|p| p.name == "libc-hello").unwrap().libc);
        assert_eq!(programs[0].source, Path::new("tests/linux/arch_prctl.c"));
    }

    #[test]
    fn syscalls_directive_is_read_from_the_source() {
        assert_eq!(syscalls_directive("int x;\n").unwrap(), None);
        let src = "/* Two processes.\n * poly-diff: syscalls=off (scheduling decides the order)\n */\n";
        assert_eq!(syscalls_directive(src).unwrap().as_deref(), Some("off"));
        assert_eq!(syscalls_directive("// poly-diff: syscalls=names\n").unwrap().as_deref(), Some("names"));
        assert!(syscalls_directive("// poly-diff: syscalls=loose\n").is_err());
        assert!(syscalls_directive("// poly-diff: off\n").is_err());
    }

    /// Two builds with the same compiler give the same bytes.
    #[test]
    fn builds_are_reproducible() {
        let cc = compiler(None);
        if compiler_version(&cc).is_err() {
            eprintln!("skipped: no C compiler");
            return;
        }
        let base = std::env::temp_dir().join(format!("poly-corpus-test-{}", std::process::id()));
        let a = build(&root(), &base.join("a"), &cc).unwrap();
        let b = build(&root(), &base.join("b"), &cc).unwrap();
        assert_eq!(a.tests, b.tests);
        assert!(a.find("hello").is_some());

        // An entry's tests in another directory build alongside, in the
        // same format; a name taken by the corpus is refused.
        let extra = base.join("brk.tests");
        let header = root().join("tests/linux/include/poly_test.h");
        let source = format!("#include \"{}\"\nint test_main(void) {{ return 7; }}\n", header.display());
        util::write(&extra.join("brk_extra.c"), &source).unwrap();
        let c = build_with(&root(), &base.join("c"), &cc, std::slice::from_ref(&extra)).unwrap();
        assert_eq!(c.find("brk_extra").unwrap().kind, "freestanding");
        util::write(&extra.join("hello.c"), &source).unwrap();
        assert!(build_with(&root(), &base.join("d"), &cc, &[extra]).unwrap_err().contains("two test programs"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
