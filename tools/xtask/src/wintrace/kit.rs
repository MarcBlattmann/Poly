//! Building the kit: the proxies, the logger and the instrumented program.

use super::*;

// ----- building the kit -------------------------------------------------------------------

/// The kit's manifest.
pub const KIT_FILE: &str = "polytrace-kit.json";
/// Where the generated workspace and its build go.
pub(super) fn work_dir() -> PathBuf {
    crate::root().join("target/wintrace")
}

fn sha256_hex(path: &Path) -> String {
    crate::sha256::file(path).unwrap_or_default()
}

fn output_of(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Build the logger and the proxies of `proxies` (`-Zbuild-std`, as ADR
/// 0075's test programs); returns the directory with the DLLs.
pub(super) fn build(proxies: &[Proxy]) -> Result<PathBuf> {
    build_with(proxies, false, false)
}

/// [`build`]; `agg` builds the counting logger.
fn build_with(proxies: &[Proxy], agg: bool, sync: bool) -> Result<PathBuf> {
    let repo = crate::root();
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let sysroot = PathBuf::from(output_of(Command::new(&rustc).current_dir(&repo).args(["--print", "sysroot"]))?);
    let vendor = sysroot.join("lib/rustlib/src/rust/library/vendor");
    if !vendor.is_dir() {
        return Err(format!("{} not found: the rust-src component is needed", vendor.display()));
    }
    let gen_dir = work_dir().join("build");
    generate_with(&gen_dir, &repo, proxies, agg, sync)?;
    let target_dir = work_dir().join(if agg {
        "cargo-agg"
    } else if sync {
        "cargo-sync"
    } else {
        "cargo"
    });
    let flags = [
        "-Clinker=rust-lld".to_string(),
        "-Clinker-flavor=lld-link".into(),
        "-Clink-arg=/NODEFAULTLIB".into(),
        "-Clink-arg=/Brepro".into(),
        "-Clink-arg=/DEBUG:NONE".into(),
        "-Cdebuginfo=0".into(),
        format!("--remap-path-prefix={}=/poly", repo.display()),
        format!("--remap-path-prefix={}=/rust", sysroot.display()),
    ]
    .join("\u{1f}");
    eprintln!("==> build polytrace.dll and {} proxy DLLs", proxies.len());
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(&repo)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_ENCODED_RUSTFLAGS", flags)
        .env_remove("RUSTFLAGS")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(gen_dir.join("Cargo.toml"))
        .args(["-Zbuild-std=core,compiler_builtins", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--target", "x86_64-pc-windows-msvc"])
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--offline")
        .arg("--config")
        .arg("source.crates-io.replace-with=\"rustsrc\"")
        .arg("--config")
        .arg(format!("source.rustsrc.directory={:?}", vendor.to_string_lossy()))
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building the tracer's DLLs failed ({status})"));
    }
    Ok(target_dir.join("x86_64-pc-windows-msvc/release"))
}

/// What [`build_kit`] made.
#[derive(Clone, Debug)]
pub struct Kit {
    pub dir: PathBuf,
    pub exe: String,
    pub proxies: Vec<Proxy>,
}

/// `trace-kit`: the instrumented copy of `exe`, its proxies and the logger
/// in `out` (emptied first).
pub fn build_kit(exe: &Path, out: &Path, data: &[String]) -> Result<Kit> {
    let bytes = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let imports = crate::winimports::imports_of(&bytes)?;
    let proxies = plan(&imports, data)?;
    let renamed = rename_imports(&bytes, |dll| {
        let low = dll.to_ascii_lowercase();
        proxies.iter().find(|p| p.real == low).map(|p| p.file.clone())
    })?;
    let built = build(&proxies)?;
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let name = exe.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    std::fs::write(out.join(&name), &renamed).map_err(|e| format!("{}: {e}", out.display()))?;
    let copy = |from: &str, to: &str| -> Result<()> {
        std::fs::copy(built.join(from), out.join(to)).map(|_| ()).map_err(|e| format!("{from}: {e}"))
    };
    copy("polytrace.dll", "polytrace.dll")?;
    for p in &proxies {
        copy(&format!("{}.dll", p.crate_name), &p.file)?;
    }
    let mut files = serde_json::Map::new();
    for f in
        std::iter::once(name.clone()).chain(["polytrace.dll".to_string()]).chain(proxies.iter().map(|p| p.file.clone()))
    {
        files.insert(f.clone(), Value::String(sha256_hex(&out.join(&f))));
    }
    let manifest = serde_json::json!({
        "format": "polytrace-kit/1",
        "program": name,
        "program_sha256": sha256_hex(exe),
        "files": files,
        "proxies": proxies.iter().map(|p| serde_json::json!({
            "dll": p.real,
            "file": p.file,
            "functions": p.funcs.iter().map(|f| serde_json::json!({
                "name": f.sym.to_string(),
                "signature": f.sig,
                "flags": f.flags,
            })).collect::<Vec<_>>(),
            "forwarded": p.data,
        })).collect::<Vec<_>>(),
        "run": "run the program in this directory; it writes polytrace.jsonl in its current directory (POLYTRACE_OUT names another file)",
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    std::fs::write(out.join(KIT_FILE), text).map_err(|e| format!("{KIT_FILE}: {e}"))?;
    Ok(Kit { dir: out.to_path_buf(), exe: name, proxies })
}

/// The Chrome kit (`trace-kit --agg`): the programs `exes` (the files of
/// one application, in one directory) get the import names of every
/// system DLL they import changed to a proxy's, and one set of proxies
/// serves them all. A DLL that is one of `exes` (the application's own),
/// is named in `skip`, or is not a `.dll` (`.drv`, `.cpl`) keeps its
/// name and is not traced. With `agg` the logger counts instead of
/// writing every call (`tools/wintrace/polytrace-agg`); with `sync` the
/// logger is `polytrace-sync` (ADR 0155).
pub fn build_kit_multi(
    exes: &[PathBuf],
    out: &Path,
    data: &[String],
    agg: bool,
    sync: bool,
    skip: &[String],
) -> Result<Kit> {
    let own: Vec<String> =
        exes.iter().filter_map(|e| e.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase())).collect();
    let traced = |low: &str| {
        low.ends_with(".dll") && !own.iter().any(|o| o == low) && !skip.iter().any(|s| s.eq_ignore_ascii_case(low))
    };
    let mut imports: Vec<Import> = Vec::new();
    let mut untraced: Vec<String> = Vec::new();
    let mut programs: Vec<(String, Vec<u8>)> = Vec::new();
    for exe in exes {
        let bytes = std::fs::read(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
        let name = exe.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
        for imp in crate::winimports::imports_of(&bytes)? {
            let low = imp.dll.to_ascii_lowercase();
            if traced(&low) {
                imports.push(imp);
            } else {
                untraced.push(low);
            }
        }
        programs.push((name, bytes));
    }
    untraced.sort();
    untraced.dedup();
    let proxies = plan_ex(&imports, data, agg || sync)?;
    let mut renamed = Vec::new();
    for (name, bytes) in &programs {
        let r = rename_imports(bytes, |dll| {
            let low = dll.to_ascii_lowercase();
            proxies.iter().find(|p| p.real == low).map(|p| p.file.clone())
        })
        .map_err(|e| format!("{name}: {e}"))?;
        renamed.push((name.clone(), r));
    }
    let built = build_with(&proxies, agg, sync)?;
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    for (name, bytes) in &renamed {
        std::fs::write(out.join(name), bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    let copy = |from: &str, to: &str| -> Result<()> {
        std::fs::copy(built.join(from), out.join(to)).map(|_| ()).map_err(|e| format!("{from}: {e}"))
    };
    copy("polytrace.dll", "polytrace.dll")?;
    for p in &proxies {
        copy(&format!("{}.dll", p.crate_name), &p.file)?;
    }
    let mut files = serde_json::Map::new();
    for f in renamed
        .iter()
        .map(|(n, _)| n.clone())
        .chain(["polytrace.dll".to_string()])
        .chain(proxies.iter().map(|p| p.file.clone()))
    {
        files.insert(f.clone(), Value::String(sha256_hex(&out.join(&f))));
    }
    let originals: Vec<Value> = exes
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.file_name().map(|n| n.to_string_lossy().into_owned()),
                "sha256": sha256_hex(e),
            })
        })
        .collect();
    let manifest = serde_json::json!({
        "format": "polytrace-kit/2",
        "mode": if agg { "agg" } else { "full" },
        "programs": originals,
        "files": files,
        "untraced_dlls": untraced,
        "proxies": proxies.iter().map(|p| serde_json::json!({
            "dll": p.real,
            "file": p.file,
            "functions": p.funcs.iter().map(|f| serde_json::json!({
                "name": f.sym.to_string(),
                "signature": f.sig,
                "flags": f.flags,
            })).collect::<Vec<_>>(),
            "forwarded": p.data,
        })).collect::<Vec<_>>(),
        "run": "overlay this directory on a copy of the application; every process writes polytrace-<pid>.jsonl (POLYTRACE_OUT names another file, %p is the process id)",
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())? + "\n";
    std::fs::write(out.join(KIT_FILE), text).map_err(|e| format!("{KIT_FILE}: {e}"))?;
    // The counting logger loads these in a confined child process, before its
    // sandbox narrows the DLL search path (`polytrace-agg`, `agg::preload`).
    let preload: String = proxies.iter().map(|p| format!("{}\n", p.file)).collect();
    std::fs::write(out.join("polytrace-preload.txt"), preload).map_err(|e| format!("polytrace-preload.txt: {e}"))?;
    let exe = renamed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ");
    Ok(Kit { dir: out.to_path_buf(), exe, proxies })
}
