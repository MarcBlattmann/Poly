//! Importers that seed the coverage map from public symbol lists.
//!
//! Only *names* (and ABI numbers where they are part of the ABI) are taken.
//! No code, signatures or behaviour is copied from any source.

use crate::model::{Entry, Layer, Os};
use std::path::Path;

pub const WINE_SPEC: &str = "wine-spec";
pub const LINUX_TBL: &str = "linux-syscall-tbl";
pub const XNU_SYSCALLS: &str = "xnu-syscalls-master";
pub const XNU_MACH_TRAPS: &str = "xnu-mach-traps";
pub const MACOS_SDK: &str = "macos-sdk";

/// One export line from a Wine `.spec` file.
#[derive(Debug, PartialEq, Eq)]
pub struct SpecExport {
    pub ordinal: Option<u32>,
    pub name: Option<String>,
    pub arch: Option<String>,
    pub syscall: bool,
    pub forward: Option<String>,
}

/// Parse one `.spec` line: `ORDINAL TYPE [-FLAGS...] NAME[(ARGS)] [TARGET]`.
pub fn parse_spec_line(line: &str) -> Option<SpecExport> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() {
        return None;
    }
    let (ordinal, rest) = line.split_once(char::is_whitespace)?;
    let ordinal = if ordinal == "@" { None } else { Some(ordinal.parse().ok()?) };
    let (kind, mut rest) = rest.trim_start().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
    if !matches!(kind, "stdcall" | "cdecl" | "varargs" | "fastcall" | "thiscall" | "extern" | "stub" | "equate") {
        return None;
    }

    let mut arch = None;
    let mut syscall = false;
    loop {
        rest = rest.trim_start();
        if !rest.starts_with('-') {
            break;
        }
        let (flag, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if let Some(a) = flag.strip_prefix("-arch=") {
            arch = Some(a.to_string());
        } else if flag == "-syscall" || flag.starts_with("-syscall=") {
            syscall = true;
        }
        rest = tail;
    }

    let end = rest.find(|c: char| c == '(' || c.is_whitespace()).unwrap_or(rest.len());
    let name = &rest[..end];
    let mut tail = &rest[end..];
    if tail.starts_with('(') {
        tail = tail.find(')').map_or("", |i| &tail[i + 1..]);
    }
    let target = tail.trim();
    let forward = (kind != "equate" && target.contains('.') && !target.contains(' ')).then(|| target.to_string());
    let name = (!name.is_empty() && name != "@").then(|| name.to_string());
    Some(SpecExport { ordinal, name, arch, syscall, forward })
}

/// Module file name for a spec file stem, or `None` if it is not a real
/// 32/64-bit Windows module.
pub fn module_name(stem: &str) -> Option<String> {
    let lower = stem.to_ascii_lowercase();
    // Wine-internal modules (winex11.drv, wined3d, winecrt0, ...).
    if lower.starts_with("wine") {
        return None;
    }
    // API-set contract DLLs are name redirections handled by the loader,
    // not code; their exports are covered by the DLLs they forward to.
    if lower.starts_with("api-ms-") || lower.starts_with("ext-ms-") {
        return None;
    }
    // 16-bit Windows modules (krnl386.exe16, *.dll16, *.vxd) don't run on 64-bit Windows.
    if lower.ends_with("16") || lower.ends_with(".vxd") {
        return None;
    }
    Some(if lower.contains('.') { lower } else { format!("{lower}.dll") })
}

pub fn wine_specs(wine_root: &Path) -> Result<Vec<Entry>, String> {
    let dlls = wine_root.join("dlls");
    let mut out = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(&dlls)
        .map_err(|e| format!("{}: {e}", dlls.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    dirs.sort();
    for dir in dirs {
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        let mut specs: Vec<_> = files
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "spec"))
            .collect();
        specs.sort();
        for spec in specs {
            let stem = spec.file_stem().unwrap().to_string_lossy().to_string();
            let Some(module) = module_name(&stem) else { continue };
            let text = std::fs::read_to_string(&spec).map_err(|e| format!("{}: {e}", spec.display()))?;
            out.extend(entries_from_spec(&module, &text));
        }
    }
    Ok(dedup(out))
}

pub fn entries_from_spec(module: &str, text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for export in text.lines().filter_map(parse_spec_line) {
        let symbol = match (&export.name, export.ordinal) {
            (Some(n), _) => n.clone(),
            (None, Some(o)) => format!("#{o}"),
            (None, None) => continue,
        };
        // Wine's own private helpers are not Windows API.
        if symbol.starts_with("__wine") || symbol.starts_with("wine_") {
            continue;
        }
        let mut e = Entry::new(Os::Windows, Layer::Api, module, &symbol, WINE_SPEC);
        e.arch = export.arch.iter().cloned().collect();
        e.forward = export.forward.clone();
        out.push(e);

        // ntdll's Nt* and win32u's NtUser*/NtGdi* stubs are the kernel boundary.
        let table = match module {
            "ntdll.dll" => Some("ntoskrnl"),
            "win32u.dll" => Some("win32k"),
            _ => None,
        };
        if let (true, Some(table)) = (export.syscall, table) {
            let mut s = Entry::new(Os::Windows, Layer::Syscall, table, &symbol, WINE_SPEC);
            s.arch = export.arch.iter().cloned().collect();
            out.push(s);
        }
    }
    out
}

/// Linux `arch/x86/entry/syscalls/syscall_64.tbl`: `NR ABI NAME [ENTRY]`.
pub fn linux_syscall_tbl(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let cols: Vec<&str> = line.split_whitespace().collect();
        let [nr, abi, name, _entry, ..] = cols[..] else { continue };
        if abi == "x32" {
            continue;
        }
        let Ok(nr) = nr.parse::<u64>() else { continue };
        let mut e = Entry::new(Os::Linux, Layer::Syscall, "x86_64", name, LINUX_TBL);
        e.number = Some(nr);
        out.push(e);
    }
    dedup(out)
}

/// XNU `bsd/kern/syscalls.master`: `NR AUDIT FILES { RET NAME(ARGS); } ...`.
pub fn xnu_syscalls_master(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(first) = line.split_whitespace().next() else { continue };
        let Ok(nr) = first.parse::<u64>() else { continue };
        let Some(open) = line.find('{') else { continue };
        let Some(paren) = line[open..].find('(') else { continue };
        let Some(name) = line[open + 1..open + paren].split_whitespace().last() else { continue };
        if name == "nosys" || name == "enosys" {
            continue;
        }
        let mut e = Entry::new(Os::Macos, Layer::Syscall, "bsd", name, XNU_SYSCALLS);
        e.number = Some(nr);
        out.push(e);
    }
    dedup(out)
}

/// XNU `osfmk/kern/syscall_sw.c`: `/* NR */ MACH_TRAP(name, ...)` lines.
pub fn xnu_mach_traps(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("/*") else { continue };
        let Some((nr, rest)) = rest.split_once("*/") else { continue };
        let Ok(nr) = nr.trim().parse::<u64>() else { continue };
        let Some(args) = rest.trim().strip_prefix("MACH_TRAP(") else { continue };
        let name = args.split(',').next().unwrap_or("").trim();
        if name.is_empty() || name == "kern_invalid" {
            continue;
        }
        let mut e = Entry::new(Os::Macos, Layer::Syscall, "mach", name, XNU_MACH_TRAPS);
        e.number = Some(nr);
        out.push(e);
    }
    dedup(out)
}

// ---------------------------------------------------------------- macOS SDK

/// What one library inside a `.tbd` file exports for macOS.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TbdLibrary {
    pub install_name: String,
    /// Raw exported symbol names (`_malloc`, `_OBJC_CLASS_$_NSObject`, ...).
    pub symbols: Vec<String>,
    /// Objective-C class names.
    pub objc_classes: Vec<String>,
}

/// Parse a text-based stub (`.tbd`): YAML versions 1–4 or JSON version 5.
/// Only exports for macOS targets are kept (not iOS, Mac Catalyst, ...).
pub fn parse_tbd(text: &str) -> Result<Vec<TbdLibrary>, String> {
    if text.trim_start().starts_with('{') { parse_tbd_json(text) } else { Ok(parse_tbd_yaml(text)) }
}

fn is_macos_target(t: &str) -> bool {
    t.ends_with("-macos") || t.ends_with("-macosx")
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| s.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(s)
}

fn flow_list(value: &str) -> Vec<String> {
    let v = value.trim();
    let inner = v.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(v);
    inner.split(',').map(unquote).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// `(indent, key, value)` per YAML line, with multi-line `[ ... ]` lists joined.
fn yaml_lines(doc: &str) -> Vec<(usize, String, String)> {
    let mut out: Vec<(usize, String, String)> = Vec::new();
    let mut open = false;
    for line in doc.lines() {
        if open {
            if let Some(last) = out.last_mut() {
                last.2.push(' ');
                last.2.push_str(line.trim());
                open = !last.2.contains(']');
            }
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut indent = line.len() - trimmed.len();
        let rest = match trimmed.strip_prefix("- ") {
            Some(r) => {
                indent += 2;
                // A new list item: mark it so blocks can be told apart.
                out.push((indent, "-".into(), String::new()));
                r
            }
            None => trimmed,
        };
        let Some((key, value)) = rest.split_once(':') else { continue };
        let value = value.trim().to_string();
        open = value.starts_with('[') && !value.contains(']');
        out.push((indent, key.trim().to_string(), value));
    }
    out
}

#[derive(Default)]
struct Block {
    targets: Vec<String>,
    symbols: Vec<String>,
    classes: Vec<String>,
}

fn parse_tbd_yaml(text: &str) -> Vec<TbdLibrary> {
    let mut docs: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with("---") || docs.is_empty() {
            docs.push(String::new());
        }
        if line.starts_with("...") {
            continue;
        }
        let d = docs.last_mut().unwrap();
        d.push_str(line);
        d.push('\n');
    }

    let mut out = Vec::new();
    for doc in docs {
        let header = doc.lines().next().unwrap_or("");
        // v1 and v2 write Objective-C class names with a leading underscore.
        let old_format = !header.contains("!tapi-tbd-v3")
            && !header.contains("!tapi-tbd\n")
            && !header.trim_end().ends_with("!tapi-tbd");
        let mut lib = TbdLibrary::default();
        let mut platform_macos = true;
        let mut section = String::new();
        let mut blocks: Vec<Block> = Vec::new();
        for (indent, key, value) in yaml_lines(&doc) {
            if indent == 0 {
                section = key.clone();
                match key.as_str() {
                    "install-name" => lib.install_name = unquote(&value).to_string(),
                    "platform" => platform_macos = matches!(unquote(&value), "macos" | "macosx"),
                    _ => {}
                }
                continue;
            }
            if section != "exports" {
                continue;
            }
            if key == "-" {
                blocks.push(Block::default());
                continue;
            }
            let Some(b) = blocks.last_mut() else { continue };
            match key.as_str() {
                "targets" => b.targets = flow_list(&value),
                "symbols" | "weak-symbols" | "weak-def-symbols" | "thread-local-symbols" => {
                    b.symbols.extend(flow_list(&value))
                }
                "objc-classes" => b.classes.extend(flow_list(&value)),
                _ => {}
            }
        }
        if lib.install_name.is_empty() {
            continue;
        }
        for b in blocks {
            let wanted =
                if b.targets.is_empty() { platform_macos } else { b.targets.iter().any(|t| is_macos_target(t)) };
            if !wanted {
                continue;
            }
            lib.symbols.extend(b.symbols);
            lib.objc_classes.extend(
                b.classes
                    .into_iter()
                    .map(|c| if old_format { c.strip_prefix('_').map(str::to_string).unwrap_or(c) } else { c }),
            );
        }
        out.push(lib);
    }
    out
}

fn parse_tbd_json(text: &str) -> Result<Vec<TbdLibrary>, String> {
    use serde_json::Value;
    let root: Value = serde_json::from_str(text).map_err(|e| format!("tbd v5: {e}"))?;
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let mut libs: Vec<&Value> = root.get("main_library").into_iter().collect();
    if let Some(more) = root.get("libraries").and_then(Value::as_array) {
        libs.extend(more);
    }
    let mut out = Vec::new();
    for lib in libs {
        let install_name = lib
            .get("install_names")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|n| n.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if install_name.is_empty() {
            continue;
        }
        let lib_targets: Vec<String> = lib
            .get("target_info")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|t| t.get("target").and_then(Value::as_str)).map(str::to_string).collect())
            .unwrap_or_default();
        let mut t = TbdLibrary { install_name, ..Default::default() };
        for group in lib.get("exported_symbols").and_then(Value::as_array).into_iter().flatten() {
            let targets = match group.get("targets") {
                Some(v) => strings(Some(v)),
                None => lib_targets.clone(),
            };
            if !targets.is_empty() && !targets.iter().any(|t| is_macos_target(t)) {
                continue;
            }
            for kind in ["data", "text"] {
                let Some(section) = group.get(kind) else { continue };
                for key in ["global", "weak", "thread_local"] {
                    t.symbols.extend(strings(section.get(key)));
                }
                t.objc_classes.extend(strings(section.get("objc_class")));
            }
        }
        out.push(t);
    }
    Ok(out)
}

/// Coverage component for a library's install name: the innermost
/// framework (`AppKit.framework`) or the dylib file name. Private
/// frameworks are not public API and return `None`.
pub fn sdk_component(install_name: &str) -> Option<String> {
    if install_name.contains("/PrivateFrameworks/") {
        return None;
    }
    if let Some(pos) = install_name.rfind(".framework/") {
        let start = install_name[..pos].rfind('/').map_or(0, |i| i + 1);
        return Some(install_name[start..pos + ".framework".len()].to_string());
    }
    install_name.rsplit('/').next().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Coverage symbol for a raw exported name, or `None` for linker metadata.
/// C symbols lose the leading underscore Mach-O adds; Objective-C class
/// symbols become `objc:ClassName`.
pub fn sdk_symbol(raw: &str) -> Option<String> {
    if raw.starts_with("$ld$") {
        return None;
    }
    for prefix in ["_OBJC_CLASS_$_", "_OBJC_METACLASS_$_"] {
        if let Some(class) = raw.strip_prefix(prefix) {
            return Some(format!("objc:{class}"));
        }
    }
    if raw.starts_with("_OBJC_IVAR_$_") || raw.starts_with("_OBJC_EHTYPE_$_") {
        return None;
    }
    Some(raw.strip_prefix('_').unwrap_or(raw).to_string())
}

pub fn entries_from_tbd(libs: &[TbdLibrary]) -> Vec<Entry> {
    let mut out = Vec::new();
    for lib in libs {
        let Some(component) = sdk_component(&lib.install_name) else { continue };
        let symbols = lib.symbols.iter().filter_map(|s| sdk_symbol(s));
        let classes = lib.objc_classes.iter().map(|c| format!("objc:{c}"));
        for symbol in symbols.chain(classes) {
            out.push(Entry::new(Os::Macos, Layer::Api, &component, &symbol, MACOS_SDK));
        }
    }
    out
}

/// Every public `.tbd` under the SDK's frameworks and `usr/lib`.
/// Symlinks are skipped: each one points at a real file that is read anyway.
pub fn macos_sdk(sdk: &Path) -> Result<Vec<Entry>, String> {
    if !sdk.join("SDKSettings.json").exists() && !sdk.join("SDKSettings.plist").exists() {
        return Err(format!("{} does not look like a macOS SDK (no SDKSettings)", sdk.display()));
    }
    let mut files = Vec::new();
    for dir in ["System/Library/Frameworks", "usr/lib"] {
        collect_tbd(&sdk.join(dir), &mut files)?;
    }
    files.sort();
    let mut out = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let libs = parse_tbd(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        out.extend(entries_from_tbd(&libs));
    }
    if out.is_empty() {
        return Err(format!("no exports found in {} .tbd files under {}", files.len(), sdk.display()));
    }
    eprintln!("read {} .tbd files", files.len());
    Ok(dedup(out))
}

fn collect_tbd(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let Ok(read) = std::fs::read_dir(dir) else { return Ok(()) };
    for entry in read {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if entry.file_name() != "PrivateFrameworks" {
                collect_tbd(&path, out)?;
            }
        } else if path.extension().is_some_and(|x| x == "tbd") {
            out.push(path);
        }
    }
    Ok(())
}

/// Collapse duplicate ids (the same export listed per architecture, or the
/// same syscall under different `#if` branches). An entry restricted to
/// some architectures widens to the union; unrestricted wins.
fn dedup(mut v: Vec<Entry>) -> Vec<Entry> {
    v.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out: Vec<Entry> = Vec::with_capacity(v.len());
    for e in v {
        match out.last_mut() {
            Some(last) if last.id == e.id => {
                if last.arch.is_empty() || e.arch.is_empty() {
                    last.arch.clear();
                } else {
                    for a in e.arch {
                        if !last.arch.contains(&a) {
                            last.arch.push(a);
                        }
                    }
                }
                last.forward = last.forward.take().or(e.forward);
                last.number = last.number.or(e.number);
            }
            _ => out.push(e),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_lines() {
        let e = parse_spec_line("@ stdcall CreateFileW(wstr long long ptr long long long)").unwrap();
        assert_eq!(e.name.as_deref(), Some("CreateFileW"));
        assert_eq!(e.forward, None);

        let e =
            parse_spec_line("@ stdcall CreateFileW(wstr long long ptr long long long) kernelbase.CreateFileW").unwrap();
        assert_eq!(e.forward.as_deref(), Some("kernelbase.CreateFileW"));

        let e = parse_spec_line("@ stdcall -syscall NtClose(long)").unwrap();
        assert!(e.syscall);

        let e = parse_spec_line("@ stdcall -syscall=0x000f -arch=win64 NtClose(long)").unwrap();
        assert!(e.syscall);
        assert_eq!(e.arch.as_deref(), Some("win64"));

        let e = parse_spec_line("@ stub -arch=i386 SomeStub").unwrap();
        assert_eq!(e.name.as_deref(), Some("SomeStub"));

        let e = parse_spec_line("12 stub -noname @").unwrap();
        assert_eq!((e.ordinal, e.name), (Some(12), None));

        let e = parse_spec_line("@ extern -arch=win32 _imp__X  # comment").unwrap();
        assert_eq!(e.name.as_deref(), Some("_imp__X"));

        assert_eq!(parse_spec_line("# just a comment"), None);
        assert_eq!(parse_spec_line(""), None);
    }

    #[test]
    fn module_names() {
        assert_eq!(module_name("kernel32").as_deref(), Some("kernel32.dll"));
        assert_eq!(module_name("winspool.drv").as_deref(), Some("winspool.drv"));
        assert_eq!(module_name("ntoskrnl.exe").as_deref(), Some("ntoskrnl.exe"));
        assert_eq!(module_name("winex11.drv"), None);
        assert_eq!(module_name("krnl386.exe16"), None);
        assert_eq!(module_name("api-ms-win-core-file-l1-1-0"), None);
    }

    #[test]
    fn spec_entries_include_syscalls_and_skip_wine_internals() {
        let text =
            "@ stdcall -syscall NtClose(long)\n@ cdecl __wine_dbg_output(str)\n@ stdcall RtlZeroMemory(ptr long)\n";
        let ids: Vec<_> = entries_from_spec("ntdll.dll", text).into_iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            [
                "windows/api/ntdll.dll/NtClose",
                "windows/syscall/ntoskrnl/NtClose",
                "windows/api/ntdll.dll/RtlZeroMemory"
            ]
        );
    }

    #[test]
    fn linux_table() {
        let text = "# comment\n0\tcommon\tread\t\t\tsys_read\n512\tx32\trt_sigaction\t\tcompat_sys_rt_sigaction\n13\t64\trt_sigaction\t\tsys_rt_sigaction\n";
        let v = linux_syscall_tbl(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "linux/syscall/x86_64/read");
        assert_eq!(v[1].number, Some(13));
    }

    #[test]
    fn xnu_tables() {
        let master = "0\tAUE_NULL\tALL\t{ int nosys(void); }   { indirect syscall }\n\
                      3\tAUE_NULL\tALL\t{ user_ssize_t read(int fd, user_addr_t cbuf, user_size_t nbyte); }\n";
        let v = xnu_syscalls_master(master);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].symbol.as_str(), v[0].number), ("read", Some(3)));

        let sw = "/* 0 */ MACH_TRAP(kern_invalid, 0, 0, NULL),\n/* 10 */ MACH_TRAP(_kernelrpc_mach_vm_allocate_trap, 4, 5, munge_wwlw),\n";
        let v = xnu_mach_traps(sw);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].number, Some(10));
    }

    #[test]
    fn dedup_widens_arch() {
        let mut a = Entry::new(Os::Windows, Layer::Api, "m.dll", "F", WINE_SPEC);
        a.arch = vec!["win32".into()];
        let mut b = a.clone();
        b.arch = vec!["win64".into()];
        let v = dedup(vec![a.clone(), b]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].arch, ["win32", "win64"]);
        let v = dedup(vec![a, Entry::new(Os::Windows, Layer::Api, "m.dll", "F", WINE_SPEC)]);
        assert!(v[0].arch.is_empty());
    }

    const TBD_V4: &str = "--- !tapi-tbd
tbd-version:     4
targets:         [ x86_64-macos, arm64-macos, arm64-maccatalyst ]
install-name:    '/System/Library/Frameworks/Demo.framework/Versions/A/Demo'
current-version: 1
exports:
  - targets:         [ x86_64-macos, arm64-macos ]
    symbols:         [ _DemoFunction, '_$s4Demo4ViewVMn', __ZN4demo3fooEv,
                       '$ld$previous$/usr/lib/libold.dylib$$1$10.0$11.0$$' ]
    objc-classes:    [ DemoWindow, DemoView ]
    objc-ivars:      [ DemoWindow._frame ]
    weak-symbols:    [ _DemoWeak ]
  - targets:         [ arm64-maccatalyst ]
    symbols:         [ _CatalystOnly ]
reexports:
  - targets:         [ x86_64-macos ]
    symbols:         [ _ReexportedElsewhere ]
undefineds:
  - targets:         [ x86_64-macos ]
    symbols:         [ _Undefined ]
--- !tapi-tbd
tbd-version:     4
targets:         [ x86_64-macos ]
install-name:    '/System/Library/Frameworks/Demo.framework/Versions/A/Frameworks/Inner.framework/Versions/A/Inner'
exports:
  - targets:         [ x86_64-macos ]
    symbols:         [ _InnerFn ]
...
";

    fn ids(entries: Vec<Entry>) -> Vec<String> {
        let mut v: Vec<String> = entries.into_iter().map(|e| e.id).collect();
        v.sort();
        v
    }

    #[test]
    fn tbd_v4() {
        let libs = parse_tbd(TBD_V4).unwrap();
        assert_eq!(libs.len(), 2);
        assert_eq!(
            ids(entries_from_tbd(&libs)),
            [
                "macos/api/Demo.framework/$s4Demo4ViewVMn",
                "macos/api/Demo.framework/DemoFunction",
                "macos/api/Demo.framework/DemoWeak",
                "macos/api/Demo.framework/_ZN4demo3fooEv",
                "macos/api/Demo.framework/objc:DemoView",
                "macos/api/Demo.framework/objc:DemoWindow",
                "macos/api/Inner.framework/InnerFn",
            ]
        );
    }

    #[test]
    fn tbd_v5_json() {
        let text = r#"{
          "tapi_tbd_version": 5,
          "main_library": {
            "target_info": [ { "target": "x86_64-macos" }, { "target": "arm64-macos" } ],
            "install_names": [ { "name": "/usr/lib/libdemo.dylib" } ],
            "exported_symbols": [
              { "data": { "global": [ "_dataSym" ], "objc_class": [ "DemoClass" ] },
                "text": { "global": [ "_textSym" ], "weak": [ "_weakSym" ] } },
              { "targets": [ "arm64-ios" ], "text": { "global": [ "_iosOnly" ] } }
            ]
          },
          "libraries": [
            { "install_names": [ { "name": "/System/Library/PrivateFrameworks/Secret.framework/Secret" } ],
              "exported_symbols": [ { "text": { "global": [ "_hidden" ] } } ] }
          ]
        }"#;
        let libs = parse_tbd(text).unwrap();
        assert_eq!(
            ids(entries_from_tbd(&libs)),
            [
                "macos/api/libdemo.dylib/dataSym",
                "macos/api/libdemo.dylib/objc:DemoClass",
                "macos/api/libdemo.dylib/textSym",
                "macos/api/libdemo.dylib/weakSym",
            ]
        );
    }

    #[test]
    fn tbd_v3_and_v2() {
        let v3 = "--- !tapi-tbd-v3\narchs: [ x86_64 ]\nplatform: macosx\ninstall-name: /usr/lib/libv3.dylib\n\
                  exports:\n  - archs: [ x86_64 ]\n    re-exports: [ /usr/lib/libother.dylib ]\n    \
                  symbols: [ _a ]\n    objc-classes: [ C3 ]\n    weak-def-symbols: [ _w ]\n...\n";
        assert_eq!(
            ids(entries_from_tbd(&parse_tbd(v3).unwrap())),
            ["macos/api/libv3.dylib/a", "macos/api/libv3.dylib/objc:C3", "macos/api/libv3.dylib/w"]
        );
        let v2 = "--- !tapi-tbd-v2\narchs: [ x86_64 ]\nplatform: macosx\ninstall-name: /usr/lib/libv2.dylib\n\
                  exports:\n  - archs: [ x86_64 ]\n    objc-classes: [ _C2 ]\n...\n";
        assert_eq!(ids(entries_from_tbd(&parse_tbd(v2).unwrap())), ["macos/api/libv2.dylib/objc:C2"]);
        let ios = "--- !tapi-tbd-v3\narchs: [ arm64 ]\nplatform: ios\ninstall-name: /usr/lib/libios.dylib\n\
                   exports:\n  - archs: [ arm64 ]\n    symbols: [ _x ]\n...\n";
        assert!(entries_from_tbd(&parse_tbd(ios).unwrap()).is_empty());
    }

    #[test]
    fn sdk_names() {
        assert_eq!(
            sdk_component("/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit").as_deref(),
            Some("AppKit.framework")
        );
        assert_eq!(sdk_component("/usr/lib/libSystem.B.dylib").as_deref(), Some("libSystem.B.dylib"));
        assert_eq!(sdk_component("/System/Library/PrivateFrameworks/X.framework/X"), None);
        assert_eq!(sdk_symbol("_OBJC_CLASS_$_NSWindow").as_deref(), Some("objc:NSWindow"));
        assert_eq!(sdk_symbol("_OBJC_IVAR_$_NSWindow._frame"), None);
        assert_eq!(sdk_symbol("$ld$hide$os10.4$_foo"), None);
        assert_eq!(sdk_symbol("_malloc").as_deref(), Some("malloc"));
    }

    #[test]
    fn sdk_walk_skips_symlinks_and_private_frameworks() {
        let sdk = std::env::temp_dir().join(format!("poly-fake-sdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sdk);
        let fw = sdk.join("System/Library/Frameworks/Demo.framework/Versions/A");
        std::fs::create_dir_all(&fw).unwrap();
        std::fs::write(sdk.join("SDKSettings.json"), "{}").unwrap();
        std::fs::write(fw.join("Demo.tbd"), TBD_V4).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("A", sdk.join("System/Library/Frameworks/Demo.framework/Versions/Current"))
                .unwrap();
        }
        let private = sdk.join("System/Library/PrivateFrameworks/Secret.framework");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("Secret.tbd"), TBD_V4.replace("Demo.framework", "Secret.framework")).unwrap();
        let v = macos_sdk(&sdk).unwrap();
        assert_eq!(v.len(), 7, "each symbol once, despite the Current symlink");
        assert!(v.iter().all(|e| !e.id.contains("Secret")));
        assert!(macos_sdk(&sdk.join("nope")).is_err());
        let _ = std::fs::remove_dir_all(&sdk);
    }
}
