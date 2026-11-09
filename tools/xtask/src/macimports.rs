//! `cargo xtask macos ...`: what a macOS app imports, and what of it Poly
//! lacks (ADR 0157).
//!
//! - `macos fetch-chrome [x64|arm64]`: download the pinned Chrome for
//!   Testing build ([`CHROME`]) into a cache under the main checkout's
//!   `target/`, checked against its size and SHA-256. Like 7-Zip (ADR 0087)
//!   it is never committed or redistributed.
//! - `macos imports <input>... [--report <file.md>]`: for every Mach-O file
//!   of each input — an `.app` bundle or any directory, a `.zip` of one, a
//!   single file, or `chrome-x64` / `chrome-arm64` for the pinned builds —
//!   the libraries it links and every symbol it imports, by name, compared
//!   with the macOS framework map in `coverage/macos/api/` (built from the
//!   SDK's `.tbd` stubs). With `--report` it writes the gap map as
//!   Markdown (`docs/macos/chrome-mac-imports.md` is the one for Chrome).
//!
//! Imports are read with `poly-macho` (ADR 0153): the chained fixups'
//! import table, or the bind and lazy-bind opcode streams of the classic
//! format, each with the library ordinal that says which `LC_LOAD_DYLIB`
//! it comes from. Nothing but load commands and import *names* is read
//! (CLEAN_ROOM.md); no code is disassembled and nothing runs.

use crate::{Result, root};
use poly_macho::{BIND_SPECIAL_DYLIB_FLAT_LOOKUP, BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE, BIND_SPECIAL_DYLIB_SELF};
use poly_macho::{CpuKind, Dylib, DylibKind, Fat, FileKind, FileType, MachO};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A pinned download.
struct Pinned {
    /// What `imports` calls it.
    name: &'static str,
    file: &'static str,
    url: &'static str,
    size: u64,
    sha256: &'static str,
}

/// Chrome for Testing Stable 157.0.8103.38 for macOS, as the owner pinned
/// it (Google's `chrome-for-testing-public` bucket, over HTTPS). The sizes
/// and SHA-256 sums are of the files downloaded on 2026-11-09.
pub const CHROME_VERSION: &str = "157.0.8103.38";
const CHROME: [Pinned; 2] = [
    Pinned {
        name: "chrome-x64",
        file: "chrome-mac-x64.zip",
        url: "https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/mac-x64/chrome-mac-x64.zip",
        size: 207_117_913,
        sha256: "c6fcc2869b0d5a421e880d7bcdabe96e4261c4100e125fc4e4daf6346474efa6",
    },
    Pinned {
        name: "chrome-arm64",
        file: "chrome-mac-arm64.zip",
        url: "https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.38/mac-arm64/chrome-mac-arm64.zip",
        size: 196_562_867,
        sha256: "3406b38b52b7d341d74450eb7b369be038c9fec9894c8b8a9e9e9f97333caf2f",
    },
];

/// `<main checkout>/target/chrome-mac/<version>/`, beside the `ntdll`
/// cache, so every worktree shares one download.
fn cache_dir() -> PathBuf {
    let ntdll = crate::ntdll::cache_dir(&root());
    ntdll.parent().map_or_else(|| root().join("target"), Path::to_path_buf).join("chrome-mac").join(CHROME_VERSION)
}

/// Whether `path` is the pinned file (size first, then the hash).
fn matches(path: &Path, p: &Pinned) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == p.size) && crate::sha256::file(path).as_deref() == Some(p.sha256)
}

/// The pinned file, downloaded into the cache unless it is there.
fn fetch(p: &Pinned) -> Result<PathBuf> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(p.file);
    if matches(&path, p) {
        return Ok(path);
    }
    eprintln!("==> downloading {}", p.url);
    let partial = path.with_extension("part");
    let fetched = Command::new("curl")
        .args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "3", "-o"])
        .arg(&partial)
        .arg(p.url)
        .status();
    if !fetched.is_ok_and(|s| s.success()) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("downloading {} failed (needs `curl`)", p.url));
    }
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    if !matches(&path, p) {
        let got = crate::sha256::file(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        return Err(format!("{} is not the pinned file (SHA-256 {}, got {got}); removed", p.file, p.sha256));
    }
    Ok(path)
}

/// `cargo xtask macos <command>`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("fetch-chrome") => {
            let which = args.get(1).map_or("all", String::as_str);
            for p in CHROME.iter().filter(|p| which == "all" || p.name.ends_with(which)) {
                let path = fetch(p)?;
                eprintln!("==> {} is Chrome for Testing {CHROME_VERSION} ({})", path.display(), p.name);
            }
            Ok(())
        }
        Some("imports") => imports(&args[1..]),
        _ => Err("usage: cargo xtask macos fetch-chrome [x64|arm64] | imports <app|zip|file|chrome-x64|chrome-arm64>... [--report <file.md>]".into()),
    }
}

/// One input of `imports`, read into memory: a name for the report and its
/// Mach-O files (path inside the input, bytes).
struct Input {
    label: String,
    /// The pinned download it is, if it is one.
    pinned: Option<&'static Pinned>,
    files: Vec<(String, Vec<u8>)>,
}

/// Whether `data` starts like a Mach-O or universal file.
fn is_macho(data: &[u8]) -> bool {
    FileKind::detect(data).is_ok()
}

/// The Mach-O files of `arg`.
fn read_input(arg: &str) -> Result<Input> {
    let pinned = CHROME.iter().find(|p| p.name == arg);
    let path = match pinned {
        Some(p) => fetch(p)?,
        None => PathBuf::from(arg),
    };
    let label = pinned.map_or_else(|| arg.to_string(), |p| format!("{} ({})", p.name, p.file));
    let mut files = Vec::new();
    if path.is_dir() {
        let mut stack = vec![path.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .filter_map(std::result::Result::ok)
                .collect();
            entries.sort_by_key(std::fs::DirEntry::path);
            for e in entries {
                // Symbolic links (a framework's `Versions/Current`) would
                // list the same files twice.
                let Ok(kind) = e.file_type() else { continue };
                if kind.is_symlink() {
                    continue;
                }
                let p = e.path();
                if kind.is_dir() {
                    stack.push(p);
                } else if let Ok(data) = std::fs::read(&p)
                    && is_macho(&data)
                {
                    let rel = p.strip_prefix(&path).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                    files.push((rel, data));
                }
            }
        }
    } else {
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if is_macho(&data) {
            files.push((path.file_name().map_or(arg.into(), |n| n.to_string_lossy().into_owned()), data));
        } else {
            for m in crate::unzip::members(&data)? {
                // Only executables and libraries can be Mach-O files worth
                // unpacking: the archive's other 600-odd members are
                // resources (`.pak`, `.lproj`, images).
                let candidate = !m.is_symlink()
                    && m.size >= 4
                    && (m.mode & 0o111 != 0 || m.name.ends_with(".dylib") || m.name.ends_with(".so"));
                if candidate {
                    let bytes = crate::unzip::read(&data, &m)?;
                    if is_macho(&bytes) {
                        files.push((m.name.clone(), bytes));
                    }
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.is_empty() {
        return Err(format!("{arg}: no Mach-O file in it"));
    }
    Ok(Input { label, pinned, files })
}

/// Where an import is looked up.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    /// A library the image loads, by its install name.
    Lib(String),
    /// The image itself (`BIND_SPECIAL_DYLIB_SELF`).
    SelfImage,
    /// The main executable (`BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE`).
    MainExecutable,
    /// Every loaded image, in order (`BIND_SPECIAL_DYLIB_FLAT_LOOKUP`).
    Flat,
    /// The weak definitions of every image (`BIND_SPECIAL_DYLIB_WEAK_LOOKUP`).
    WeakLookup,
    /// An ordinal that names no dependency (a broken file).
    Bad(i32),
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Lib(n) => f.write_str(n),
            Source::SelfImage => f.write_str("(the image itself)"),
            Source::MainExecutable => f.write_str("(the main executable)"),
            Source::Flat => f.write_str("(flat lookup)"),
            Source::WeakLookup => f.write_str("(weak definitions, any image)"),
            Source::Bad(o) => write!(f, "(bad ordinal {o})"),
        }
    }
}

/// One slice of one Mach-O file.
struct Image {
    /// The path inside the input, and the slice's architecture.
    path: String,
    arch: &'static str,
    file_type: String,
    /// `LC_BUILD_VERSION` (or `LC_VERSION_MIN_MACOSX`): minimum OS and SDK.
    min_os: Option<String>,
    sdk: Option<String>,
    deps: Vec<(String, DylibKind)>,
    /// Import name → where it is looked up and whether it may be missing.
    imports: BTreeMap<(Source, String), bool>,
    chained: bool,
}

fn arch_name(kind: CpuKind) -> &'static str {
    match kind {
        CpuKind::X86_64 => "x86_64",
        CpuKind::Arm64 => "arm64",
        CpuKind::Arm64e => "arm64e",
        CpuKind::Other => "other",
    }
}

fn file_type_name(t: FileType) -> String {
    match t {
        FileType::Execute => "executable".into(),
        FileType::Dylib => "dylib".into(),
        FileType::Bundle => "bundle".into(),
        FileType::Dylinker => "dylinker".into(),
        other => format!("{other:?}"),
    }
}

fn source_of(m: &MachO<'_>, ordinal: i32) -> Source {
    match ordinal {
        BIND_SPECIAL_DYLIB_SELF => Source::SelfImage,
        BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE => Source::MainExecutable,
        BIND_SPECIAL_DYLIB_FLAT_LOOKUP => Source::Flat,
        -3 => Source::WeakLookup,
        o if o > 0 => m.dylib_by_ordinal(o as u32).map_or(Source::Bad(o), |d: Dylib<'_>| Source::Lib(d.name.into())),
        o => Source::Bad(o),
    }
}

/// Read one thin slice.
fn image(path: &str, data: &[u8]) -> Result<Image> {
    let m = MachO::parse(data).map_err(|e| format!("{path}: {e:?}"))?;
    let s = m.summary().map_err(|e| format!("{path}: {e:?}"))?;
    let mut imports = BTreeMap::new();
    let mut add = |src: Source, name: &[u8], weak: bool| {
        let e = imports.entry((src, String::from_utf8_lossy(name).into_owned())).or_insert(weak);
        // Weak only if every reference is weak.
        *e &= weak;
    };
    if let Some(cf) = m.chained_fixups().map_err(|e| format!("{path}: chained fixups: {e:?}"))? {
        for i in cf.imports() {
            let i = i.map_err(|e| format!("{path}: chained imports: {e:?}"))?;
            add(source_of(&m, i.lib_ordinal), i.name, i.weak_import);
        }
    }
    if let Some(info) = m.dyld_info().map_err(|e| format!("{path}: dyld info: {e:?}"))? {
        for b in info.binds().chain(info.lazy_binds()) {
            let b = b.map_err(|e| format!("{path}: binds: {e:?}"))?;
            add(source_of(&m, b.ordinal), b.symbol, b.is_weak_import());
        }
        for b in info.weak_binds() {
            let b = b.map_err(|e| format!("{path}: weak binds: {e:?}"))?;
            add(Source::WeakLookup, b.symbol, true);
        }
    }
    Ok(Image {
        path: path.to_string(),
        arch: arch_name(s.cpu.kind()),
        file_type: file_type_name(s.file_type),
        min_os: s.platform.map(|p| p.minos.to_string()),
        sdk: s.platform.map(|p| p.sdk.to_string()),
        deps: m.dylibs().map(|d| (d.name.to_string(), d.kind)).collect(),
        imports,
        chained: s.has_chained_fixups,
    })
}

/// Every slice of every Mach-O file of an input.
fn images(input: &Input) -> Result<Vec<Image>> {
    let mut out = Vec::new();
    for (path, data) in &input.files {
        match FileKind::detect(data).map_err(|e| format!("{path}: {e:?}"))? {
            FileKind::Fat | FileKind::Fat64 => {
                let fat = Fat::parse(data).map_err(|e| format!("{path}: {e:?}"))?;
                for a in fat.arches() {
                    let slice = fat.slice(&a).map_err(|e| format!("{path}: {e:?}"))?;
                    out.push(image(path, slice)?);
                }
            }
            FileKind::Thin64 => out.push(image(path, data)?),
            other => eprintln!("==> {path}: {other:?}, not read"),
        }
    }
    Ok(out)
}

/// The macOS framework map of `coverage/macos/api/`: component (file name
/// without `.json`, e.g. `AppKit.framework`) → its symbols, and symbol →
/// the components that export it.
struct SdkMap {
    by_component: BTreeMap<String, BTreeSet<String>>,
    by_symbol: BTreeMap<String, Vec<String>>,
    /// The coverage map's status of each symbol (absent: `missing`).
    status: BTreeMap<(String, String), String>,
}

fn sdk_map() -> Result<SdkMap> {
    let dir = root().join("coverage/macos/api");
    let mut map = SdkMap { by_component: BTreeMap::new(), by_symbol: BTreeMap::new(), status: BTreeMap::new() };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for f in files {
        let component = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let entries: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        let mut set = BTreeSet::new();
        for e in entries.as_array().into_iter().flatten() {
            let Some(sym) = e.get("symbol").and_then(|s| s.as_str()) else { continue };
            set.insert(sym.to_string());
            map.by_symbol.entry(sym.to_string()).or_default().push(component.clone());
            if let Some(st) = e.get("status").and_then(|s| s.as_str()) {
                map.status.insert((component.clone(), sym.to_string()), st.to_string());
            }
        }
        map.by_component.insert(component, set);
    }
    Ok(map)
}

/// The coverage map's component for an install name: `AppKit.framework`
/// for `/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit`,
/// `libSystem.B.dylib` for `/usr/lib/libSystem.B.dylib`; `None` for a
/// library of the app itself (`@rpath/...`, `@executable_path/...`).
fn component(install_name: &str) -> Option<String> {
    if install_name.starts_with('@') {
        return None;
    }
    if let Some(i) = install_name.find(".framework/") {
        let start = install_name[..i].rfind('/').map_or(0, |s| s + 1);
        return Some(format!("{}.framework", &install_name[start..i]));
    }
    install_name.rsplit('/').next().map(str::to_string)
}

/// The name an import has in the coverage map: without the C compiler's
/// leading underscore, and `objc:<class>` for the Objective-C class
/// symbols (`_OBJC_CLASS_$_NSView`, its metaclass and exception type).
fn map_name(import: &str) -> String {
    for prefix in ["_OBJC_CLASS_$_", "_OBJC_METACLASS_$_", "_OBJC_EHTYPE_$_"] {
        if let Some(class) = import.strip_prefix(prefix) {
            return format!("objc:{class}");
        }
    }
    import.strip_prefix('_').unwrap_or(import).to_string()
}

/// Umbrella libraries whose symbols live in the libraries they re-export:
/// a symbol imported from them is known if any component has it.
fn is_umbrella(component: &str) -> bool {
    matches!(
        component,
        "libSystem.B.dylib"
            | "Cocoa.framework"
            | "ApplicationServices.framework"
            | "CoreServices.framework"
            | "Carbon.framework"
            | "Quartz.framework"
            | "Accelerate.framework"
    )
}

/// The area of the report a symbol of a component belongs to.
fn area(component: &str, symbol: &str) -> &'static str {
    let mach_prefixes = [
        "mach_",
        "task_",
        "thread_",
        "vm_",
        "host_",
        "bootstrap_",
        "xpc_",
        "semaphore_",
        "_kernelrpc_",
        "mig_",
        "NDR_record",
        "processor_",
        "exception_",
        "voucher_",
        "audit_token_",
        "launch_",
    ];
    let c = component;
    if c == "libSystem.B.dylib" || c.starts_with("libsystem_") || c == "libxpc.dylib" || c == "libdispatch.dylib" {
        let s = symbol.strip_prefix('_').unwrap_or(symbol);
        if mach_prefixes.iter().any(|p| s.starts_with(p)) || c == "libxpc.dylib" {
            return "Mach / XPC";
        }
        return "libSystem / BSD";
    }
    match c {
        "CoreFoundation.framework" => "CoreFoundation",
        "Foundation.framework" => "Foundation",
        "AppKit.framework" | "Cocoa.framework" | "Carbon.framework" => "AppKit",
        "CoreGraphics.framework"
        | "CoreText.framework"
        | "IOSurface.framework"
        | "ImageIO.framework"
        | "QuartzCore.framework"
        | "CoreImage.framework"
        | "ColorSync.framework"
        | "CoreVideo.framework"
        | "ApplicationServices.framework" => "CoreGraphics / CoreText / IOSurface",
        "Security.framework" | "LocalAuthentication.framework" | "SecurityInterface.framework" => "Security",
        "Metal.framework" | "MetalKit.framework" | "OpenGL.framework" | "IOSurfaceRoot" => "Metal / OpenGL",
        "libc++.1.dylib" | "libc++abi.dylib" | "libobjc.A.dylib" => "C++ and Objective-C runtimes",
        _ => "other",
    }
}

const AREAS: [&str; 10] = [
    "libSystem / BSD",
    "Mach / XPC",
    "C++ and Objective-C runtimes",
    "CoreFoundation",
    "Foundation",
    "AppKit",
    "CoreGraphics / CoreText / IOSurface",
    "Security",
    "Metal / OpenGL",
    "other",
];

/// What the map says of one import from a system library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Known {
    /// In the component the import names (or, for an umbrella, anywhere).
    Yes,
    /// Not there, but another component exports it.
    Elsewhere,
    /// No component exports it (private, or newer than the SDK map).
    No,
}

fn known(map: &SdkMap, comp: &str, name: &str) -> Known {
    let in_comp = map.by_component.get(comp).is_some_and(|s| s.contains(name));
    let anywhere = map.by_symbol.contains_key(name);
    if in_comp || (is_umbrella(comp) && anywhere) {
        Known::Yes
    } else if anywhere {
        Known::Elsewhere
    } else {
        Known::No
    }
}

/// `cargo xtask macos imports <input>... [--report <file>]`.
fn imports(args: &[String]) -> Result {
    let mut inputs = Vec::new();
    let mut report: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--report" {
            report = Some(PathBuf::from(it.next().ok_or("--report needs a file")?));
        } else {
            inputs.push(a.clone());
        }
    }
    if inputs.is_empty() {
        return Err("imports: name an app, a .zip, a Mach-O file, chrome-x64 or chrome-arm64".into());
    }
    let map = sdk_map()?;
    let mut analysed = Vec::new();
    for a in &inputs {
        let input = read_input(a)?;
        let imgs = images(&input)?;
        eprintln!("==> {}: {} Mach-O files, {} slices", input.label, input.files.len(), imgs.len());
        analysed.push((input, imgs));
    }
    let text = render(&map, &analysed);
    match report {
        Some(path) => {
            let path = if path.is_absolute() { path } else { root().join(path) };
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
            eprintln!("==> wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// A system library's imports over all images of an input: symbol →
/// (weak everywhere, the images that import it).
type LibImports = BTreeMap<String, BTreeMap<String, (bool, BTreeSet<String>)>>;

fn system_imports(imgs: &[Image]) -> LibImports {
    let mut out: LibImports = BTreeMap::new();
    for img in imgs {
        for ((src, name), weak) in &img.imports {
            if let Source::Lib(lib) = src
                && component(lib).is_some()
            {
                let e = out.entry(lib.clone()).or_default().entry(name.clone()).or_insert((true, BTreeSet::new()));
                e.0 &= *weak;
                e.1.insert(short(&img.path));
            }
        }
    }
    out
}

/// A Mach-O file's path without the bundle's long prefix.
fn short(path: &str) -> String {
    let p = path.split_once(".app/").map_or(path, |(_, rest)| rest);
    p.replace("Contents/Frameworks/Google Chrome for Testing Framework.framework/Versions/157.0.8103.38/", "Framework/")
}

fn render(map: &SdkMap, analysed: &[(Input, Vec<Image>)]) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# What Chrome for macOS imports: the gap map\n");
    let _ = writeln!(
        o,
        "Generated by `cargo xtask macos imports {}` (ADR 0157). Do not edit by hand.\n",
        analysed.iter().map(|(i, _)| i.pinned.map_or(i.label.as_str(), |p| p.name)).collect::<Vec<_>>().join(" ")
    );
    let _ = writeln!(
        o,
        "Every Mach-O file of each input was read with `poly-macho`: its load commands, the libraries it links, and \
         the *names* of the symbols it imports (the chained fixups' import table, or the classic bind and lazy-bind \
         streams). The names are compared with the macOS framework map in `coverage/macos/api/` (the SDK's `.tbd` \
         stubs). Nothing else of the files was read (CLEAN_ROOM.md).\n"
    );
    for (input, imgs) in analysed {
        let _ = writeln!(o, "## {}\n", input.label);
        if let Some(p) = input.pinned {
            let _ = writeln!(o, "- Source: <{}>", p.url);
            let _ = writeln!(o, "- Size {} bytes, SHA-256 `{}`", p.size, p.sha256);
        }
        let arches: BTreeSet<_> = imgs.iter().map(|i| i.arch).collect();
        let main = imgs.iter().find(|i| i.file_type == "executable" && !i.path.contains("Helper"));
        let _ = writeln!(
            o,
            "- {} Mach-O files, {} slices; architectures: {}",
            input.files.len(),
            imgs.len(),
            join(&arches)
        );
        if let Some(m) = main {
            let _ = writeln!(
                o,
                "- Main executable `{}`: {}, minimum macOS {} (`LC_BUILD_VERSION`), SDK {}, {}",
                short(&m.path),
                m.arch,
                m.min_os.as_deref().unwrap_or("?"),
                m.sdk.as_deref().unwrap_or("?"),
                if m.chained { "chained fixups" } else { "classic binds" }
            );
        }
        let mins: BTreeSet<_> = imgs.iter().filter_map(|i| i.min_os.clone()).collect();
        let _ = writeln!(o, "- Minimum macOS over all images: {}", join(&mins));
        let x86 = imgs.iter().filter(|i| i.arch == "x86_64").count();
        let _ = writeln!(
            o,
            "- Real x86_64 code: {}\n",
            if x86 == imgs.len() {
                "every image is an x86_64 slice, so this build runs on x86 PCs without the arm64 translator"
            } else if x86 > 0 {
                "some images have an x86_64 slice"
            } else {
                "none; every image is arm64 (needs the translator on x86 PCs)"
            }
        );

        let _ = writeln!(o, "### The images\n");
        let _ = writeln!(o, "| File | Arch | Kind | Min macOS | Fixups | Libraries | Imports |");
        let _ = writeln!(o, "|---|---|---|---|---|---|---|");
        for i in imgs {
            let _ = writeln!(
                o,
                "| `{}` | {} | {} | {} | {} | {} | {} |",
                short(&i.path),
                i.arch,
                i.file_type,
                i.min_os.as_deref().unwrap_or("?"),
                if i.chained { "chained" } else { "classic" },
                i.deps.len(),
                i.imports.len()
            );
        }
        let _ = writeln!(o);

        // The app's own libraries and what they are loaded as.
        let mut internal: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut system: BTreeMap<String, (DylibKind, BTreeSet<String>)> = BTreeMap::new();
        for i in imgs {
            for (d, kind) in &i.deps {
                if component(d).is_some() {
                    system.entry(d.clone()).or_insert((*kind, BTreeSet::new())).1.insert(short(&i.path));
                } else {
                    internal.entry(d.clone()).or_default().insert(short(&i.path));
                }
            }
        }
        let _ = writeln!(o, "### Libraries of the app itself\n");
        for (d, by) in &internal {
            let _ = writeln!(o, "- `{d}`, loaded by {}", join_code(by));
        }
        let _ = writeln!(o);

        let sys = system_imports(imgs);
        let _ = writeln!(o, "### System libraries and frameworks\n");
        let _ = writeln!(
            o,
            "Per library: how it is linked, how many distinct symbols the app imports from it, how many of them the \
             SDK map knows for that library, and the coverage map's status (Poly implements none yet: all are \
             `missing`). \"Not in the map\" is mostly a gap of the map, not of macOS: its seed skips the private \
             libraries a public framework re-exports (AppKit's UIFoundation, AudioToolbox's AudioToolboxCore, \
             AVFoundation's AVFCore and AVFCapture) and the `reexports:` lists of the SDK stubs (libsystem_c's \
             `memcpy`, `strlen`, ...); see ADR 0157.\n"
        );
        let _ = writeln!(
            o,
            "| Library | Linked as | Area | Imports | Known to the map | Elsewhere in the map | Not in the map | Status |"
        );
        let _ = writeln!(o, "|---|---|---|---|---|---|---|---|");
        let mut area_counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for (lib, (kind, _)) in &system {
            let comp = component(lib).unwrap_or_default();
            let syms = sys.get(lib).cloned().unwrap_or_default();
            let (mut yes, mut elsewhere, mut no) = (0, 0, 0);
            let mut green = 0;
            for name in syms.keys() {
                let n = map_name(name);
                match known(map, &comp, &n) {
                    Known::Yes => yes += 1,
                    Known::Elsewhere => elsewhere += 1,
                    Known::No => no += 1,
                }
                if map.status.get(&(comp.clone(), n.clone())).is_some_and(|s| s == "green") {
                    green += 1;
                }
                area_counts.entry(area(&comp, name)).or_default().0 += 1;
            }
            area_counts.entry(area(&comp, "")).or_default().1 += 1;
            let _ = writeln!(
                o,
                "| `{}` | {} | {} | {} | {} | {} | {} | {} |",
                lib,
                kind_name(*kind),
                area(&comp, ""),
                syms.len(),
                yes,
                elsewhere,
                no,
                if green == 0 { "missing".to_string() } else { format!("{green} green") }
            );
        }
        let _ = writeln!(o);
        let _ = writeln!(o, "### By area\n");
        let _ = writeln!(o, "| Area | Imported symbols | Libraries (by their main area) |");
        let _ = writeln!(o, "|---|---|---|");
        for a in AREAS {
            let (s, l) = area_counts.get(a).copied().unwrap_or_default();
            let _ = writeln!(o, "| {a} | {s} | {l} |");
        }
        let total: usize = sys.values().map(BTreeMap::len).sum();
        let _ = writeln!(o, "| **all** | **{total}** | **{}** |\n", system.len());

        // Imports not from a named system library.
        let mut special: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for i in imgs {
            for (src, name) in i.imports.keys() {
                if !matches!(src, Source::Lib(l) if component(l).is_some()) {
                    special.entry(src.to_string()).or_default().insert(name.clone());
                }
            }
        }
        if !special.is_empty() {
            let _ = writeln!(o, "### Imports bound elsewhere\n");
            for (src, names) in &special {
                let _ = writeln!(o, "- {src}: {} symbols", names.len());
            }
            let _ = writeln!(o);
        }
    }

    // The symbol lists, of the first input; the others as differences.
    if let Some((first, imgs)) = analysed.first() {
        let sys = system_imports(imgs);
        let _ = writeln!(o, "## The symbols, by area and library ({})\n", first.label);
        let _ = writeln!(
            o,
            "Each symbol as the binary names it. `weak` = a weak import (may be missing at run time); `not in map` = \
             the SDK map has no such symbol for that library (`elsewhere` = another library has it).\n"
        );
        for a in AREAS {
            let in_area = |(lib, syms): &(&String, &BTreeMap<String, (bool, BTreeSet<String>)>)| {
                let comp = component(lib).unwrap_or_default();
                syms.keys().any(|n| area(&comp, n) == a)
            };
            let libs: Vec<_> = sys.iter().filter(in_area).collect();
            if libs.is_empty() {
                continue;
            }
            let _ = writeln!(o, "### {a}\n");
            for (lib, syms) in libs {
                let comp = component(lib).unwrap_or_default();
                let chosen: Vec<_> = syms.iter().filter(|(n, _)| area(&comp, n) == a).collect();
                if chosen.is_empty() {
                    continue;
                }
                let _ = writeln!(o, "<details><summary><code>{lib}</code>: {} symbols</summary>\n", chosen.len());
                for (name, (weak, _)) in chosen {
                    let mut notes = Vec::new();
                    if *weak {
                        notes.push("weak");
                    }
                    match known(map, &comp, &map_name(name)) {
                        Known::Yes => {}
                        Known::Elsewhere => notes.push("elsewhere"),
                        Known::No => notes.push("not in map"),
                    }
                    let note = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
                    let _ = writeln!(o, "- `{name}`{note}");
                }
                let _ = writeln!(o, "\n</details>\n");
            }
        }
        let first_set: BTreeSet<(String, String)> =
            sys.iter().flat_map(|(l, s)| s.keys().map(move |n| (l.clone(), n.clone()))).collect();
        for (input, imgs) in &analysed[1..] {
            let other = system_imports(imgs);
            let set: BTreeSet<(String, String)> =
                other.iter().flat_map(|(l, s)| s.keys().map(move |n| (l.clone(), n.clone()))).collect();
            let _ = writeln!(o, "## {} compared with {}\n", input.label, first.label);
            for (title, diff) in [
                ("Only here", set.difference(&first_set).collect::<Vec<_>>()),
                ("Only there", first_set.difference(&set).collect::<Vec<_>>()),
            ] {
                let _ = writeln!(o, "<details><summary>{title}: {} symbols</summary>\n", diff.len());
                for (l, n) in diff {
                    let _ = writeln!(o, "- `{n}` from `{l}`");
                }
                let _ = writeln!(o, "\n</details>\n");
            }
        }
    }
    o
}

fn kind_name(k: DylibKind) -> &'static str {
    match k {
        DylibKind::Load => "load",
        DylibKind::Weak => "weak",
        DylibKind::Reexport => "re-export",
        DylibKind::Lazy => "lazy",
        DylibKind::Upward => "upward",
        DylibKind::Id => "id",
    }
}

fn join<T: std::fmt::Display>(items: &BTreeSet<T>) -> String {
    items.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

fn join_code(items: &BTreeSet<String>) -> String {
    items.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_of_install_names() {
        let c = |s: &str| component(s);
        assert_eq!(
            c("/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit").as_deref(),
            Some("AppKit.framework")
        );
        assert_eq!(c("/System/Library/Frameworks/Metal.framework/Metal").as_deref(), Some("Metal.framework"));
        assert_eq!(c("/usr/lib/libSystem.B.dylib").as_deref(), Some("libSystem.B.dylib"));
        assert_eq!(c("/usr/lib/system/libsystem_c.dylib").as_deref(), Some("libsystem_c.dylib"));
        assert_eq!(c("@rpath/Foo.framework/Foo"), None);
        assert_eq!(c("@executable_path/../Frameworks/libx.dylib"), None);
    }

    #[test]
    fn names_as_the_map_has_them() {
        assert_eq!(map_name("_write"), "write");
        assert_eq!(map_name("___stack_chk_fail"), "__stack_chk_fail");
        assert_eq!(map_name("_OBJC_CLASS_$_NSView"), "objc:NSView");
        assert_eq!(map_name("_OBJC_METACLASS_$_NSObject"), "objc:NSObject");
        assert_eq!(map_name("_OBJC_EHTYPE_$_NSException"), "objc:NSException");
    }

    #[test]
    fn areas() {
        assert_eq!(area("libSystem.B.dylib", "_mach_msg"), "Mach / XPC");
        assert_eq!(area("libSystem.B.dylib", "_xpc_connection_create"), "Mach / XPC");
        assert_eq!(area("libSystem.B.dylib", "_pthread_create"), "libSystem / BSD");
        assert_eq!(area("AppKit.framework", "_NSApp"), "AppKit");
        assert_eq!(area("IOSurface.framework", "_IOSurfaceCreate"), "CoreGraphics / CoreText / IOSurface");
        assert_eq!(area("Metal.framework", "_MTLCreateSystemDefaultDevice"), "Metal / OpenGL");
        assert_eq!(area("IOKit.framework", "_IOServiceMatching"), "other");
    }
}
