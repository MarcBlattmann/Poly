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
}
