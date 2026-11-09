//! `cargo xtask win chrome-trace --calls <windows-trace.json>`: compare Poly's call trace of
//! Chrome with a Windows trace *by caller* (ADR 0155).
//!
//! The reduction `poly-chrometrace/1` has no call sites. A call-site-annotated trace does: every
//! call is keyed by the module and offset of its return address (`chrome_elf.dll+0x8e18c`), so
//! the same code makes the same call on both systems and the calls can be paired one by one
//! (the n-th call of function F from caller K on Windows with the n-th on Poly), whatever
//! happened between them. For each pair the comparison looks at the result, the last error and
//! the values the call read (`in`) and wrote (`out`).
//!
//! Both sides are traces of the `polytrace-sync` kit (`tools/wintrace/polytrace-sync`, with
//! `dump.rs`), so the records have the same shape: `rm` / `ro` (the caller's module and offset;
//! for a trace without them the caller is made from `ra` and the PE headers of the modules in
//! Chrome's directory), `ret`, `err`, and `out`, an array of what the call left in memory
//! (strings, structures, registry data, handle and pointer classes; see `dump.rs`). Poly's file
//! is `target/chrome-win/stage1.ptrace.jsonl`; Windows' is a `polytrace.jsonl` of the same
//! kit, or a JSON document with a `calls` array of such records
//! (`specs/windows/apps/chrome/traces/s1-chrome-elf-init.trace.json`).
//!
//! Machine-specific fields (processor counts, address-space bounds, critical-section internals)
//! and volatile values (the performance counter) are left out of the comparison; handle values
//! compare as "not 0, not -1", small results (lengths, booleans, status codes) exactly.
//!
//! Report, in this order: the **frontier** (the first call, in Windows' order, that Poly never
//! made: where Poly's run diverges or stops), then **mismatches** between paired calls, then
//! calls only Poly made.

use crate::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// A module of the program: its name (lower case) and where Poly maps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub base: u64,
    pub size: u64,
}

/// `ImageBase` and `SizeOfImage` of a PE32+ file, the file name as the module name.
pub fn module_of_pe(name: &str, bytes: &[u8]) -> Option<Module> {
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let pe = u32_at(0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    if u16::from_le_bytes(bytes.get(opt..opt + 2)?.try_into().ok()?) != 0x20B {
        return None;
    }
    let base = u64::from_le_bytes(bytes.get(opt + 24..opt + 32)?.try_into().ok()?);
    Some(Module { name: name.to_ascii_lowercase(), base, size: u64::from(u32_at(opt + 56)?) })
}

/// The program's own modules in `dir` (the ones the kit traces calls *from*).
pub fn modules_in(dir: &Path) -> Vec<Module> {
    ["chrome.exe", "chrome_elf.dll", "chrome.dll"]
        .iter()
        .filter_map(|n| module_of_pe(n, &std::fs::read(dir.join(n)).ok()?))
        .collect()
}

/// `module+0xoffset` for a return address, if it is in one of `modules`.
pub fn caller_key(modules: &[Module], ra: u64) -> Option<String> {
    let m = modules.iter().find(|m| (m.base..m.base + m.size).contains(&ra))?;
    Some(format!("{}+{:#x}", m.name, ra - m.base))
}

/// One call, either side.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub key: String,
    pub name: String,
    pub ret: Option<u64>,
    pub err: Option<u32>,
    pub out: Value,
    /// Poly's sequence number (0 for Windows').
    pub seq: u64,
}

fn num(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|i| i as u64)),
        Value::String(s) => s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()),
        _ => None,
    }
}

/// Poly's calls that were made from one of `modules`.
pub fn poly_calls(text: &str, modules: &[Module]) -> Vec<Call> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let Some(name) = v["fn"].as_str() else { continue };
        let Some(key) = record_key(&v, modules) else { continue };
        out.push(Call {
            key,
            name: name.to_string(),
            ret: num(&v["ret"]),
            err: v["err"].as_u64().map(|e| e as u32),
            out: v["out"].clone(),
            seq: v["seq"].as_u64().unwrap_or(0),
        });
    }
    out
}

/// The caller key of a record: the logger's `"rm"` / `"ro"` (module name and offset), else
/// `ra` resolved with `modules`, else the document's `"caller"`.
fn record_key(v: &Value, modules: &[Module]) -> Option<String> {
    if let (Some(m), Some(o)) = (v["rm"].as_str(), num(&v["ro"])) {
        return Some(format!("{}+{:#x}", m.to_ascii_lowercase(), o));
    }
    if let Some(k) = v["caller"].as_str() {
        return Some(normal_key(k));
    }
    caller_key(modules, num(&v["ra"])?)
}

/// The calls of a Windows call-site trace (see the module documentation).
pub fn windows_calls(doc: &Value, process: &str) -> Result<Vec<Call>> {
    let calls = doc
        .get("calls")
        .or_else(|| doc["process_types"].get(process).and_then(|p| p.get("calls")))
        .and_then(Value::as_array)
        .ok_or("no `calls` array in the Windows trace")?;
    let mut out = Vec::new();
    for c in calls {
        let Some(key) = record_key(c, &[]) else { continue };
        let Some(name) = c["fn"].as_str().or_else(|| c["name"].as_str()) else { continue };
        out.push(Call {
            key,
            name: name.to_string(),
            ret: num(&c["ret"]),
            err: num(c.get("err").or_else(|| c.get("last_error")).unwrap_or(&Value::Null)).map(|e| e as u32),
            out: c["out"].clone(),
            seq: 0,
        });
    }
    Ok(out)
}

/// `Module.DLL+0X8E18C` -> `module.dll+0x8e18c`.
fn normal_key(k: &str) -> String {
    let k = k.to_ascii_lowercase();
    match k.split_once(['+', '!']) {
        Some((m, off)) => match u64::from_str_radix(off.trim_start_matches("0x"), 16) {
            Ok(o) => format!("{m}+{o:#x}"),
            Err(_) => k,
        },
        None => k,
    }
}

// ----- comparing values ---------------------------------------------------------------------

/// Fields that are the machine's, not the program's: never compared.
const IGNORED: &[&str] = &["cpus", "level", "revision", "mask", "minaddr", "maxaddr", "res2", "dbg", "sem", "spin"];

/// Functions whose written values are the moment's: never compared.
const VOLATILE: &[&str] = &[
    "QueryPerformanceCounter",
    "GetSystemTimeAsFileTime",
    "GetSystemTimePreciseAsFileTime",
    "GetTickCount",
    "GetTickCount64",
];

fn lower_norm(s: &str) -> String {
    crate::chrometrace::normalise(s).to_ascii_lowercase()
}

/// Differences of one value on Poly (`b`) from Windows' (`a`), as `path: windows … poly …`.
pub fn diff_value(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::String(x), Value::String(y)) => {
            if lower_norm(x) != lower_norm(y) {
                out.push(format!("{path}: Windows {x:?}, Poly {y:?}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, av) in x {
                if IGNORED.contains(&k.as_str()) {
                    continue;
                }
                match y.get(k) {
                    Some(bv) => diff_value(&format!("{path}.{k}"), av, bv, out),
                    None => out.push(format!("{path}.{k}: Windows has it, Poly has not")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: Windows {a}, Poly {b}"));
            } else {
                for (i, (av, bv)) in x.iter().zip(y).enumerate() {
                    diff_value(&format!("{path}[{i}]"), av, bv, out);
                }
            }
        }
        _ => {
            if a != b {
                out.push(format!("{path}: Windows {a}, Poly {b}"));
            }
        }
    }
}

/// Whether the two results are alike: small values (lengths, counts, booleans, status codes)
/// exactly; anything bigger (a handle, a pointer, a module base) only as "not 0 / not -1".
pub fn ret_alike(name: &str, win: Option<u64>, poly: Option<u64>) -> bool {
    match (win, poly) {
        (None, None) => true,
        (Some(w), Some(p)) => {
            let class = |v: u64| match v as u32 {
                0 => 0,
                u32::MAX => 1,
                x if x >= 0x8000_0000 => 2,
                _ => 3,
            };
            if !returns_handle(name) && w <= 0xFFFF && p <= 0xFFFF
                || w >= 0x8000_0000 && w <= u64::from(u32::MAX)
                || w == u64::MAX
            {
                (w as u32) == (p as u32)
            } else {
                class(w) == class(p)
            }
        }
        // A `void` function has no result on either side; one with a result against one
        // without is a difference of the kit's table, not of the system.
        _ => true,
    }
}

/// Functions whose result is a handle or a module / heap base: its value is the system's own.
fn returns_handle(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "CreateFile",
        "CreateMutex",
        "CreateEvent",
        "CreateSemaphore",
        "CreateThread",
        "CreateIoCompletionPort",
        "CreateNamedPipe",
        "CreateJobObject",
        "CreateWaitableTimer",
        "OpenProcess",
        "OpenThread",
        "OpenEvent",
        "OpenMutex",
        "GetStdHandle",
        "GetCurrentProcess",
        "GetCurrentThread",
        "FindFirstFile",
        "HeapCreate",
        "GetProcessHeap",
        "LoadLibrary",
        "GetModuleHandle",
        "GetProcAddress",
        "VirtualAlloc",
        "MapViewOfFile",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

fn err_alike(win: Option<u32>, poly: Option<u32>) -> bool {
    win.unwrap_or(0) == poly.unwrap_or(0)
}

/// The report lines: frontier first, then mismatches, then Poly-only calls.
pub fn compare_calls(poly: &[Call], win: &[Call]) -> Vec<String> {
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut by_key: BTreeMap<(&str, &str), Vec<&Call>> = BTreeMap::new();
    for c in poly {
        by_key.entry((c.key.as_str(), c.name.as_str())).or_default().push(c);
    }
    let mut frontier = None;
    let mut mismatches = Vec::new();
    for (i, w) in win.iter().enumerate() {
        let k = (w.key.as_str(), w.name.as_str());
        let n = seen.entry(k).or_insert(0);
        let nth = *n;
        *n += 1;
        let Some(p) = by_key.get(&k).and_then(|v| v.get(nth)) else {
            if frontier.is_none() {
                frontier = Some(format!(
                    "frontier: Windows call #{i} {}() from {} (ret {}) was never made by Poly",
                    w.name,
                    w.key,
                    w.ret.map_or("-".into(), |r| format!("{r:#x}"))
                ));
            }
            continue;
        };
        let mut why = Vec::new();
        if !ret_alike(&w.name, w.ret, p.ret) {
            why.push(format!(
                "result: Windows {}, Poly {}",
                w.ret.map_or("-".into(), |r| format!("{r:#x}")),
                p.ret.map_or("-".into(), |r| format!("{r:#x}"))
            ));
        }
        if !err_alike(w.err, p.err) {
            why.push(format!("last error: Windows {}, Poly {}", w.err.unwrap_or(0), p.err.unwrap_or(0)));
        }
        if w.out.is_array() && !VOLATILE.contains(&w.name.as_str()) {
            diff_value("out", &w.out, &p.out, &mut why);
        }
        if !why.is_empty() {
            mismatches.push(format!(
                "mismatch: {}() from {} (Windows call #{i}, Poly seq {}): {}",
                w.name,
                w.key,
                p.seq,
                why.join("; ")
            ));
        }
    }
    let mut lines = Vec::new();
    lines.extend(frontier);
    lines.extend(mismatches);
    for (k, v) in &by_key {
        for p in v.iter().skip(seen.get(k).copied().unwrap_or(0)) {
            lines.push(format!("poly only: {}() from {} (seq {})", p.name, p.key, p.seq));
        }
    }
    lines
}

/// Calls whose written values look like a missing implementation on Poly: an empty string where
/// a getter wrote something, a `STARTUPINFO` without desktop or title.
pub fn suspects(poly: &[Call]) -> Vec<String> {
    fn empty(v: &Value, path: &str, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for (k, x) in o {
                    match (k.as_str(), x) {
                        ("w" | "a" | "u" | "us" | "s", Value::String(t)) if t.is_empty() => {
                            out.push(format!("{path}.{k} is empty"));
                        }
                        ("desktop" | "title", Value::Null) => out.push(format!("{path}.{k} is NULL")),
                        _ => empty(x, &format!("{path}.{k}"), out),
                    }
                }
            }
            Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| empty(x, &format!("{path}[{i}]"), out)),
            _ => {}
        }
    }
    let mut lines = Vec::new();
    for c in poly.iter().filter(|c| !c.name.contains("Free")) {
        let mut why = Vec::new();
        empty(&c.out, "out", &mut why);
        if !why.is_empty() {
            lines.push(format!("suspect: {}() from {} (seq {}): {}", c.name, c.key, c.seq, why.join("; ")));
        }
    }
    lines
}

/// Print the report for `poly` (the Poly trace's text), `modules` and a Windows document.
pub fn report(poly_text: &str, modules: &[Module], win_doc: Option<&Value>, process: &str) -> Result<String> {
    let poly = poly_calls(poly_text, modules);
    let mut s = String::new();
    writeln!(s, "{} Poly calls made from {:?}", poly.len(), modules.iter().map(|m| &m.name).collect::<Vec<_>>())
        .map_err(|e| e.to_string())?;
    if let Some(doc) = win_doc {
        let win = windows_calls(doc, process)?;
        writeln!(s, "{} Windows calls", win.len()).map_err(|e| e.to_string())?;
        for l in compare_calls(&poly, &win) {
            writeln!(s, "{l}").map_err(|e| e.to_string())?;
        }
    }
    for l in suspects(&poly) {
        writeln!(s, "{l}").map_err(|e| e.to_string())?;
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mods() -> Vec<Module> {
        vec![
            Module { name: "chrome.exe".into(), base: 0x1_4000_0000, size: 0x41b000 },
            Module { name: "chrome_elf.dll".into(), base: 0x1_8000_0000, size: 0x2f0000 },
        ]
    }

    #[test]
    fn a_return_address_becomes_module_and_offset() {
        assert_eq!(caller_key(&mods(), 0x1_8008_e18c).as_deref(), Some("chrome_elf.dll+0x8e18c"));
        assert_eq!(caller_key(&mods(), 0x1_4003_cfcf).as_deref(), Some("chrome.exe+0x3cfcf"));
        assert_eq!(caller_key(&mods(), 0x7ff0_0000_0000), None);
        assert_eq!(normal_key("Chrome_Elf.DLL+0X8E18C"), "chrome_elf.dll+0x8e18c");
    }

    #[test]
    fn a_pe_header_gives_base_and_size() {
        let mut b = vec![0u8; 0x200];
        b[0x3c] = 0x80;
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let opt = 0x80 + 24;
        b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        b[opt + 24..opt + 32].copy_from_slice(&0x1_8000_0000u64.to_le_bytes());
        b[opt + 56..opt + 60].copy_from_slice(&0x2f0000u32.to_le_bytes());
        assert_eq!(
            module_of_pe("chrome_elf.DLL", &b),
            Some(Module { name: "chrome_elf.dll".into(), base: 0x1_8000_0000, size: 0x2f0000 })
        );
        assert_eq!(module_of_pe("x", &b[..0x50]), None);
    }

    fn poly_line(seq: u64, f: &str, rm: &str, ro: u64, ret: u64, extra: &str) -> String {
        format!(
            r#"{{"seq":{seq},"t":0,"dll":"kernel32.dll","fn":"{f}","args":[],"ra":"0x1","rm":"{rm}","ro":"{ro:#x}","ret":{ret}{extra}}}"#
        )
    }

    #[test]
    fn calls_pair_by_caller_and_occurrence() {
        let poly = [
            poly_line(
                1,
                "GetModuleFileNameW",
                "chrome_elf.dll",
                0x67a09,
                34,
                r#","out":[{"a":1,"w":"app/chrome.exe"}]"#,
            ),
            poly_line(
                2,
                "GetCommandLineW",
                "chrome_elf.dll",
                0x6c22d,
                0x810448,
                r#","out":[{"a":"ret","w":"c.exe --a"}]"#,
            ),
            poly_line(3, "CreateMutexW", "chrome_elf.dll", 0x8e300, 0x14, ""),
        ]
        .join(
            "
",
        );
        let win = json!({"calls": [
            {"rm": "chrome_elf.dll", "ro": "0x67a09", "fn": "GetModuleFileNameW", "ret": 34,
             "out": [{"a": 1, "w": "app/chrome.exe"}]},
            {"rm": "chrome_elf.dll", "ro": "0x6c22d", "fn": "GetCommandLineW", "ret": "0x7ff6a0001000",
             "out": [{"a": "ret", "w": "c.exe --b"}]},
            {"caller": "chrome_elf.dll+0x8e2c0", "fn": "VerifyVersionInfoW", "ret": 1},
            {"rm": "chrome_elf.dll", "ro": "0x8e300", "fn": "CreateMutexW", "ret": "0x1f0"}
        ]});
        let lines = report(&poly, &mods(), Some(&win), "browser").unwrap();
        assert!(
            lines.contains("frontier: Windows call #2 VerifyVersionInfoW() from chrome_elf.dll+0x8e2c0"),
            "{lines}"
        );
        assert!(lines.contains("mismatch: GetCommandLineW() from chrome_elf.dll+0x6c22d"), "{lines}");
        assert!(lines.contains("--a"), "{lines}");
        assert!(!lines.contains("GetModuleFileNameW() from"), "equal calls are not reported: {lines}");
        assert!(!lines.contains("CreateMutexW"), "handles only compare as non-zero: {lines}");
    }

    #[test]
    fn results_compare_exactly_when_small_and_by_class_when_big() {
        assert!(ret_alike("X", Some(34), Some(34)));
        assert!(!ret_alike("X", Some(34), Some(35)));
        assert!(ret_alike("CreateMutexW", Some(0x1f0), Some(0x14)));
        assert!(!ret_alike("X", Some(0x1_0000_1000), Some(0)));
        assert!(!ret_alike("X", Some(0xc000_0034), Some(0xc000_0035)));
        assert!(ret_alike("X", Some(u64::MAX), Some(u64::from(u32::MAX))));
        assert!(!ret_alike("X", Some(u64::MAX), Some(0)));
    }

    #[test]
    fn out_values_compare_after_normalising_and_machine_fields_are_ignored() {
        let mut d = Vec::new();
        diff_value(
            "out",
            &json!([{"a": 0, "sysinfo": {"arch": 9, "page": 4096, "cpus": 32, "level": 25, "maxaddr": "0x1"}}]),
            &json!([{"a": 0, "sysinfo": {"arch": 9, "page": 4096, "cpus": 4, "level": 6, "maxaddr": "0x2"}}]),
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        diff_value("out", &json!([{"w": "Program Files/A"}]), &json!([{"w": "program files/a"}]), &mut d);
        assert!(d.is_empty(), "{d:?}");
        diff_value("out", &json!([{"osvi": {"major": 10}}]), &json!([{"osvi": {"major": 6}}]), &mut d);
        assert_eq!(d.len(), 1, "{d:?}");
        diff_value("out", &json!([{"c": "handle"}, {"u32": 1}]), &json!([{"c": "handle"}]), &mut d);
        assert_eq!(d.len(), 2, "{d:?}");
    }

    #[test]
    fn the_caller_comes_from_rm_ro_or_from_the_return_address() {
        let a = json!({"rm": "Chrome_Elf.DLL", "ro": "0x8e1f5"});
        assert_eq!(record_key(&a, &[]).as_deref(), Some("chrome_elf.dll+0x8e1f5"));
        let b = json!({"ra": "0x18008e1f5"});
        assert_eq!(record_key(&b, &mods()).as_deref(), Some("chrome_elf.dll+0x8e1f5"));
        assert_eq!(record_key(&b, &[]), None);
    }

    #[test]
    fn suspects_are_empty_strings_and_missing_startup_names() {
        let poly = [
            poly_line(
                1,
                "GetStartupInfoW",
                "chrome_elf.dll",
                0x17b41,
                0,
                r#","out":[{"a":0,"startupinfo":{"cb":104,"desktop":null,"title":null}}]"#,
            ),
            poly_line(2, "GetUserNameW", "chrome_elf.dll", 0x17b50, 1, r#","out":[{"a":0,"w":""}]"#),
            poly_line(3, "GetEnvironmentVariableW", "chrome_elf.dll", 0x17b60, 4, r#","out":[{"a":1,"w":"abc"}]"#),
        ]
        .join(
            "
",
        );
        let s = suspects(&poly_calls(&poly, &mods()));
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[0].contains("desktop is NULL") && s[0].contains("title is NULL"));
        assert!(s[1].contains("GetUserNameW"));
    }
}
