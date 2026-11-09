//! `cargo xtask win chrome-trace`: compare Poly's call trace of Chrome with what Windows
//! recorded (ADR 0155; the follow-up the gap map of `spec-chrome-win` names).
//!
//! The Poly side is the trace a kit (`cargo xtask chrome-win run --kit <files>`) writes:
//! `polytrace/1` records, one per call of an imported function, in order. The Windows side is
//! the committed reduction `specs/windows/apps/chrome/traces/<stage>.trace.json`
//! (`poly-chrometrace/1`): per process type, the functions called in both of two runs, in the
//! order of their first call (`rank`), with counts, whether any call returned 0/NULL/-1
//! (`ok` / `mixed` / `never-ok`), the last errors seen, and the names the file, registry,
//! module and process functions were given.
//!
//! What the reduction can say is limited (no sequence, one process type at a time), so the
//! report is a list of *differences*, earliest Poly call first:
//!
//! - **fails on Poly**: Windows never saw the function return 0/NULL/-1 (`ok`) and Poly's call
//!   did; or Windows' calls all returned 0 (`never-ok`: success for an `HRESULT`, `NTSTATUS` or
//!   `LSTATUS`) and Poly's did not.
//! - **new error**: the call set a last error Windows never recorded for the function.
//! - **new name**: a file, key, module or pipe name Windows never used with the function.
//! - **skipped**: a function Windows called *before* the last one Poly reached (by first-call
//!   rank) that Poly never called; only functions the kit traced count (`--kit`).
//!
//! The first line of the report is the divergence to look at first.

use crate::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One record of Poly's trace.
#[derive(Clone, Debug, PartialEq)]
pub struct PolyCall {
    pub seq: u64,
    pub thread: u64,
    /// Lower case, `kernel32.dll`.
    pub dll: String,
    pub name: String,
    pub args: Vec<Value>,
    /// The return value as a number; `None` for `void` and for calls that do not return.
    pub ret: Option<u64>,
    pub err: Option<u32>,
    /// The caller's return address (`ra`, written when the logger runs with `POLYTRACE_SYNC`).
    pub ra: Option<u64>,
}

/// How Windows' calls of a function ended (`poly-chrometrace/1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every result was non-zero and not -1.
    Ok,
    Mixed,
    /// Every result was 0 (or -1).
    NeverOk,
}

/// A function Windows called, from the reduction.
#[derive(Clone, Debug, PartialEq)]
pub struct WinCall {
    pub dll: String,
    pub name: String,
    pub min: u64,
    pub max: u64,
    pub status: Status,
    pub errors: Vec<u32>,
    pub rank: u64,
}

/// A name a function was given on Windows, normalised.
pub type WinObject = (String, String);

fn parse_ret(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64().or_else(|| n.as_i64().map(|i| i as u64)),
        Value::String(s) => s.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()),
        _ => None,
    }
}

/// The records of a `polytrace/1` file (lines that are not records are skipped).
pub fn parse_poly(text: &str) -> Vec<PolyCall> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let (Some(seq), Some(dll), Some(name)) = (v["seq"].as_u64(), v["dll"].as_str(), v["fn"].as_str()) else {
            continue;
        };
        out.push(PolyCall {
            seq,
            thread: v["t"].as_u64().unwrap_or(0),
            dll: dll.to_ascii_lowercase(),
            name: name.to_string(),
            args: v["args"].as_array().cloned().unwrap_or_default(),
            ret: parse_ret(&v["ret"]),
            err: v["err"].as_u64().map(|e| e as u32),
            ra: parse_ret(&v["ra"]),
        });
    }
    out
}

/// The calls and objects of one process type in a `poly-chrometrace/1` document.
pub fn parse_windows(doc: &Value, process: &str) -> Result<(Vec<WinCall>, Vec<WinObject>)> {
    let p = doc["process_types"].get(process).ok_or_else(|| {
        let known: Vec<&str> =
            doc["process_types"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
        format!("no process type {process:?} in the trace (have {known:?})")
    })?;
    let mut calls = Vec::new();
    for c in p["calls"].as_array().ok_or("no calls")? {
        let c = c.as_array().ok_or("a call is not an array")?;
        if c.len() < 7 {
            return Err("a call has fewer than 7 fields".into());
        }
        let status = match c[4].as_str() {
            Some("ok") => Status::Ok,
            Some("never-ok") => Status::NeverOk,
            _ => Status::Mixed,
        };
        calls.push(WinCall {
            dll: c[0].as_str().unwrap_or("").to_ascii_lowercase(),
            name: c[1].as_str().unwrap_or("").to_string(),
            min: c[2].as_u64().unwrap_or(0),
            max: c[3].as_u64().unwrap_or(0),
            status,
            errors: c[5]
                .as_array()
                .map(|a| a.iter().filter_map(|e| e.as_u64().map(|e| e as u32)).collect())
                .unwrap_or_default(),
            rank: c[6].as_u64().unwrap_or(0),
        });
    }
    let objects = p["objects"]
        .as_array()
        .map(|a| {
            a.iter().filter_map(|o| Some((o.get(0)?.as_str()?.to_string(), o.get(1)?.as_str()?.to_string()))).collect()
        })
        .unwrap_or_default();
    Ok((calls, objects))
}

/// The functions a kit traced (`polytrace-kit.json`): `(dll, name)`, the dll in lower case.
pub fn kit_scope(kit: &Value) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for p in kit["proxies"].as_array().into_iter().flatten() {
        let dll = p["dll"].as_str().unwrap_or("").to_ascii_lowercase();
        for f in p["functions"].as_array().into_iter().flatten() {
            if let Some(n) = f["name"].as_str() {
                out.insert((dll.clone(), n.to_string()));
            }
        }
    }
    out
}

/// What kind of difference a [`Finding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    FailsOnPoly,
    NewError,
    NewName,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The Poly record it is about (`None` for a function Poly never called).
    pub seq: Option<u64>,
    pub kind: Kind,
    pub text: String,
}

/// Machine- and run-independent form of a name (the reduction's `norm_str`): the app and
/// profile directories, GUIDs, SIDs, long hex runs and numbers.
pub fn normalise(s: &str) -> String {
    let mut x = s.to_string();
    for (from, to) in [
        ("C:\\Program Files\\Chrome", "<app>"),
        ("C:\\poly-test\\profile", "<profile>"),
        ("C:\\poly-test", "<pages>"),
        ("C:\\Users\\Poly", "<home>"),
    ] {
        while let Some(at) = x.to_ascii_lowercase().find(&from.to_ascii_lowercase()) {
            x.replace_range(at..at + from.len(), to);
        }
    }
    let mut out = String::new();
    let b = x.as_bytes();
    let mut i = 0;
    while i < b.len() {
        // `{8-4-4-4-12}` GUIDs.
        if b[i] == b'{' && i + 38 <= b.len() && b[i + 37] == b'}' && b[i + 9] == b'-' && b[i + 14] == b'-' {
            out.push_str("{<guid>}");
            i += 38;
            continue;
        }
        // Domain SIDs.
        if x[i..].starts_with("S-1-5-21-") {
            let end = x[i + 9..].find(|c: char| !(c.is_ascii_digit() || c == '-')).map_or(x.len(), |e| i + 9 + e);
            out.push_str("S-1-5-21-<domain>");
            i = end;
            continue;
        }
        // Runs of hex digits of 8 or more, and of digits of 3 or more.
        let run = x[i..].find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(x.len() - i);
        let prev_alnum = i > 0 && b[i - 1].is_ascii_alphanumeric();
        if run >= 8 && !prev_alnum {
            let after = x[i + run..].chars().next();
            if !after.is_some_and(|c| c.is_ascii_alphanumeric()) || run >= 12 {
                out.push_str("<hex>");
                i += run;
                continue;
            }
        }
        let digits = x[i..].find(|c: char| !c.is_ascii_digit()).unwrap_or(x.len() - i);
        if digits >= 3 {
            out.push_str("<n>");
            i += digits;
            continue;
        }
        let ch = x[i..].chars().next().expect("in range");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Which argument of a function names the object it works on, and whether the call's first
/// argument is a registry root (the reduction's `STRING_FNS`).
fn name_arg(function: &str) -> Option<usize> {
    Some(match function {
        "CreateFileW"
        | "FindFirstFileW"
        | "FindFirstFileExW"
        | "GetFileAttributesW"
        | "GetFileAttributesExW"
        | "CreateDirectoryW"
        | "DeleteFileW"
        | "RemoveDirectoryW"
        | "MoveFileExW"
        | "CopyFileW"
        | "ReplaceFileW"
        | "GetFullPathNameW"
        | "GetLongPathNameW"
        | "SetCurrentDirectoryW"
        | "CreateNamedPipeW"
        | "WaitNamedPipeW"
        | "LoadLibraryW"
        | "LoadLibraryExW"
        | "LoadLibraryA"
        | "LoadLibraryExA"
        | "GetModuleHandleW"
        | "GetEnvironmentVariableW"
        | "SetEnvironmentVariableW"
        | "RegisterWindowMessageW" => 0,
        "RegOpenKeyExW" | "RegCreateKeyExW" | "RegQueryValueExW" | "RegGetValueW" | "RegSetValueExW"
        | "RegDeleteValueW" | "CreateJobObjectW" | "OpenEventW" | "GetProcAddress" | "CreateProcessW" => 1,
        "CreateMutexW" | "OpenFileMappingW" => 2,
        "CreateEventW" | "CreateSemaphoreW" => 3,
        "CreateFileMappingW" => 5,
        _ => return None,
    })
}

/// The registry roots as the reduction names them (`ROOTS`); any other first argument is a key.
fn reg_root(arg: &Value) -> &'static str {
    match arg.as_str() {
        Some("0xffffffff80000000") => "HKCR",
        Some("0xffffffff80000001") => "HKCU",
        Some("0xffffffff80000002") => "HKLM",
        Some("0xffffffff80000003") => "HKU",
        Some("0xffffffff80000005") => "HKCC",
        _ => "key",
    }
}

/// The name the call was given, raw (`Reg*` get the root or `key` in front, as in the reduction).
fn string_arg(call: &PolyCall) -> Option<String> {
    let i = name_arg(&call.name)?;
    let a = call.args.get(i)?;
    let s = a["w"].as_str().or_else(|| a["a"].as_str())?;
    if call.name.starts_with("Reg") {
        return Some(format!("{}\\{s}", reg_root(call.args.first()?)));
    }
    Some(s.to_string())
}

/// Functions whose Learn page gives them no return value: what `rax` holds after them means
/// nothing.
const VOID_FUNCTIONS: &[&str] = &[
    "InitializeSListHead",
    "OutputDebugStringW",
    "OutputDebugStringA",
    "SetLastError",
    "RtlInitUnicodeString",
    "Sleep",
    "EnterCriticalSection",
    "LeaveCriticalSection",
    "DeleteCriticalSection",
    "GetSystemTimeAsFileTime",
    "GetStartupInfoW",
    "GetStartupInfoA",
    "AcquireSRWLockExclusive",
    "AcquireSRWLockShared",
    "ReleaseSRWLockExclusive",
    "ReleaseSRWLockShared",
    "InitializeSRWLock",
    "InitializeConditionVariable",
    "WakeConditionVariable",
    "WakeAllConditionVariable",
    "ExitProcess",
    "ExitThread",
    "GetSystemInfo",
    "GetNativeSystemInfo",
    "RtlFreeUnicodeString",
    "RtlCaptureContext",
    "RaiseException",
    "FlsFree_",
];

/// Whether a result counts as a failure for the reduction's classes: 0, or -1 (as 32 or 64 bits).
fn is_zero_or_minus_one(v: u64) -> bool {
    v == 0 || v == u64::MAX || v == u64::from(u32::MAX)
}

/// Compare. `scope`: only functions in it are considered missing (the kit's); `None` considers
/// every function Windows called.
pub fn compare(
    poly: &[PolyCall],
    win: &[WinCall],
    objects: &[WinObject],
    scope: Option<&BTreeSet<(String, String)>>,
) -> Vec<Finding> {
    let by_name: BTreeMap<(&str, &str), &WinCall> =
        win.iter().map(|w| ((w.dll.as_str(), w.name.as_str()), w)).collect();
    let mut objs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (f, n) in objects {
        objs.entry(f.as_str()).or_default().insert(n.as_str());
    }
    let mut findings: Vec<Finding> = Vec::new();
    // Per function: Poly's calls, to classify.
    let mut seen: BTreeMap<(&str, &str), (u64, u32, u32, u32)> = BTreeMap::new(); // first seq, calls, zero/-1, nonzero
    for c in poly {
        let key = (c.dll.as_str(), c.name.as_str());
        let e = seen.entry(key).or_insert((c.seq, 0, 0, 0));
        e.1 += 1;
        if let Some(r) = c.ret {
            if is_zero_or_minus_one(r) {
                e.2 += 1;
            } else {
                e.3 += 1;
            }
        }
        let Some(w) = by_name.get(&key) else { continue };
        let label = format!("{}!{}", c.dll, c.name);
        // 1. Results.
        if let Some(r) = c.ret.filter(|_| !VOID_FUNCTIONS.contains(&c.name.as_str())) {
            let bad = match w.status {
                Status::Ok => is_zero_or_minus_one(r),
                // Only a value that looks like a failure code counts (a status code with the high
                // bit, or a small error number): the result of a function whose signature the
                // kit does not know may be anything.
                Status::NeverOk => r != 0 && r <= u64::from(u32::MAX) && !(0x1_0000..0x8000_0000).contains(&r),
                Status::Mixed => false,
            };
            if bad && !findings.iter().any(|f| f.kind == Kind::FailsOnPoly && f.text.starts_with(&label)) {
                let want =
                    if w.status == Status::Ok { "never 0/NULL/-1" } else { "always 0 (success for a status code)" };
                findings.push(Finding {
                    seq: Some(c.seq),
                    kind: Kind::FailsOnPoly,
                    text: format!(
                        "{label} returned {r:#x}{} (Windows: {want}; args {})",
                        c.err.map_or(String::new(), |e| format!(", last error {e}")),
                        brief_args(&c.args)
                    ),
                });
            }
        }
        // 2. Errors.
        if let Some(e) = c.err
            && e != 0
            && !w.errors.contains(&e)
            && !findings.iter().any(|f| f.kind == Kind::NewError && f.text.starts_with(&format!("{label} set")))
        {
            findings.push(Finding {
                seq: Some(c.seq),
                kind: Kind::NewError,
                text: format!("{label} set last error {e}; Windows saw {:?} (args {})", w.errors, brief_args(&c.args)),
            });
        }
        // 3. Names.
        if let Some(s) = string_arg(c)
            && let Some(known) = objs.get(c.name.as_str())
        {
            let n = if c.name.starts_with("Reg") {
                let (root, rest) = s.split_once('\\').unwrap_or(("key", &s));
                format!("{root}\\{}", normalise(rest))
            } else {
                normalise(&s)
            };
            if !known.contains(n.as_str())
                && !findings.iter().any(|f| f.kind == Kind::NewName && f.text.contains(&format!("{n:?}")))
            {
                findings.push(Finding {
                    seq: Some(c.seq),
                    kind: Kind::NewName,
                    text: format!("{label} was given {n:?}, which Windows never gave it"),
                });
            }
        }
    }
    // 4. Skipped functions.
    // Where Poly's trace ended: the rank of the last call it made that Windows also made.
    let reached: u64 =
        poly.iter().rev().find_map(|c| by_name.get(&(c.dll.as_str(), c.name.as_str())).map(|w| w.rank)).unwrap_or(0);
    let mut skipped: Vec<&WinCall> = win
        .iter()
        .filter(|w| w.rank < reached)
        .filter(|w| !seen.contains_key(&(w.dll.as_str(), w.name.as_str())))
        .filter(|w| scope.is_none_or(|s| s.contains(&(w.dll.clone(), w.name.clone()))))
        .collect();
    skipped.sort_by_key(|w| w.rank);
    for w in skipped {
        findings.push(Finding {
            seq: None,
            kind: Kind::Skipped,
            text: format!(
                "{}!{} (Windows' call number {}, {}-{} calls) was never called; Poly's trace reached call number {reached}",
                w.dll, w.name, w.rank, w.min, w.max
            ),
        });
    }
    findings.sort_by_key(|f| (f.seq.unwrap_or(u64::MAX), f.kind));
    findings
}

fn brief_args(args: &[Value]) -> String {
    let mut out = String::new();
    for (i, a) in args.iter().take(4).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match a {
            Value::String(s) => out.push_str(s),
            Value::Object(o) => {
                let v = o.get("w").or_else(|| o.get("a")).and_then(Value::as_str).unwrap_or("?");
                let _ = write!(out, "{v:?}");
            }
            v => {
                let _ = write!(out, "{v}");
            }
        }
    }
    out
}

/// The text of the report.
pub fn report(findings: &[Finding], poly_calls: usize, distinct: usize) -> String {
    let mut out = format!("Poly's trace: {poly_calls} calls of {distinct} functions\n");
    if findings.is_empty() {
        out.push_str("no difference from Windows' recording\n");
        return out;
    }
    out.push_str("first divergence:\n");
    let mut shown = 0;
    for f in findings {
        let tag = match f.kind {
            Kind::FailsOnPoly => "fails on Poly",
            Kind::NewError => "new error",
            Kind::NewName => "new name",
            Kind::Skipped => "skipped",
        };
        let at = f.seq.map_or(String::from("      -"), |s| format!("{s:>7}"));
        let _ = writeln!(out, "  {at}  {tag:13} {}", f.text);
        shown += 1;
        if shown >= 40 {
            let _ = writeln!(out, "  ... {} more", findings.len() - shown);
            break;
        }
    }
    out
}

const USAGE: &str = "usage: cargo xtask win chrome-trace <poly.ptrace.jsonl> [--stage s1] [--process browser] [--kit polytrace-kit.json] [--all]\n\
                     default file: target/chrome-win/stage1.ptrace.jsonl; --kit defaults to target/chrome-kit/polytrace-kit.json; \
                     --all counts every function Windows called as missing, not only the kit's";

/// `cargo xtask win chrome-trace ...`.
pub fn command(args: &[String]) -> Result {
    let root = crate::root();
    let (mut file, mut stage, mut process, mut kit, mut all) =
        (None, "s1".to_string(), "browser".to_string(), None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--stage" => stage = it.next().ok_or(USAGE)?.clone(),
            "--process" => process = it.next().ok_or(USAGE)?.clone(),
            "--kit" => kit = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--all" => all = true,
            s if !s.starts_with("--") && file.is_none() => file = Some(PathBuf::from(s)),
            _ => return Err(USAGE.into()),
        }
    }
    let file = file.unwrap_or_else(|| root.join("target/chrome-win/stage1.ptrace.jsonl"));
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let poly = parse_poly(&text);
    let trace_path = root.join(format!("specs/windows/apps/chrome/traces/{stage}.trace.json"));
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(&trace_path).map_err(|e| format!("{}: {e}", trace_path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let (win, objects) = parse_windows(&doc, &process)?;
    let kit_path = kit.unwrap_or_else(|| root.join("target/chrome-kit/polytrace-kit.json"));
    let scope = if all { None } else { read_kit(&kit_path) };
    let findings = compare(&poly, &win, &objects, scope.as_ref());
    let distinct = poly.iter().map(|c| (&c.dll, &c.name)).collect::<BTreeSet<_>>().len();
    print!("{}", report(&findings, poly.len(), distinct));
    Ok(())
}

fn read_kit(path: &Path) -> Option<BTreeSet<(String, String)>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(kit_scope(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn win_doc() -> Value {
        json!({"process_types": {"browser": {
            "calls": [
                ["kernel32.dll", "CreateFileW", 3, 3, "mixed", [2], 0],
                ["ntdll.dll", "NtCreateKey", 37, 37, "never-ok", [], 1],
                ["kernel32.dll", "GetVersionExW", 3, 3, "ok", [], 2],
                ["version.dll", "VerQueryValueW", 13, 13, "mixed", [1813], 3],
                ["kernel32.dll", "CreateMutexW", 2, 2, "ok", [], 4],
                ["kernel32.dll", "SetEvent", 9, 9, "ok", [], 5]
            ],
            "objects": [["CreateFileW", "<app>\\chrome_elf.dll"], ["CreateFileW", "<profile>\\Default\\History"]]
        }}})
    }

    fn poly_text() -> String {
        [
            r#"{"format":"polytrace/1"}"#,
            r#"{"seq":0,"t":0,"dll":"kernel32.dll","fn":"CreateFileW","args":[{"w":"C:\\Program Files\\Chrome\\chrome_elf.dll"},1],"ret":"0x14"}"#,
            r#"{"seq":1,"t":0,"dll":"ntdll.dll","fn":"NtCreateKey","args":["0x1"],"ret":"0xc0000033"}"#,
            r#"{"seq":2,"t":0,"dll":"kernel32.dll","fn":"GetVersionExW","args":["0x1"],"ret":0,"err":87}"#,
            r#"{"seq":3,"t":0,"dll":"kernel32.dll","fn":"CreateFileW","args":[{"w":"C:\\Windows\\win.ini"}],"ret":"0xffffffffffffffff","err":2}"#,
            r#"{"seq":4,"t":0,"dll":"version.dll","fn":"VerQueryValueW","args":["0x1"],"ret":0,"err":1813}"#,
            r#"{"seq":5,"t":0,"dll":"kernel32.dll","fn":"SetEvent","args":["0x1"],"ret":1}"#,
        ]
        .join("\n")
    }

    #[test]
    fn records_and_reductions_parse() {
        let poly = parse_poly(&poly_text());
        assert_eq!(poly.len(), 6);
        assert_eq!(poly[0].ret, Some(0x14));
        assert_eq!(poly[1].ret, Some(0xc000_0033));
        assert_eq!(poly[3].ret, Some(u64::MAX));
        assert_eq!(poly[2].err, Some(87));
        let (win, objects) = parse_windows(&win_doc(), "browser").unwrap();
        assert_eq!(win.len(), 6);
        assert_eq!(win[1].status, Status::NeverOk);
        assert_eq!(objects.len(), 2);
        assert!(parse_windows(&win_doc(), "gpu-process").unwrap_err().contains("browser"));
    }

    #[test]
    fn the_first_divergence_is_the_earliest_difference() {
        let poly = parse_poly(&poly_text());
        let (win, objects) = parse_windows(&win_doc(), "browser").unwrap();
        let f = compare(&poly, &win, &objects, None);
        let kinds: Vec<(Option<u64>, Kind)> = f.iter().map(|f| (f.seq, f.kind)).collect();
        // NtCreateKey: Windows always 0, Poly 0xc0000033. GetVersionExW: ok on Windows, 0 on Poly.
        // A name Windows never used. An error for GetVersionExW. CreateMutexW was skipped.
        assert_eq!(f[0].seq, Some(1));
        assert!(f[0].text.contains("ntdll.dll!NtCreateKey returned 0xc0000033"), "{}", f[0].text);
        assert!(kinds.contains(&(Some(2), Kind::FailsOnPoly)));
        assert!(kinds.contains(&(Some(2), Kind::NewError)));
        assert!(kinds.contains(&(Some(3), Kind::NewName)));
        // CreateFileW's error 2 is one Windows saw; VerQueryValueW's 1813 too.
        assert!(!kinds.contains(&(Some(3), Kind::NewError)));
        assert!(!kinds.contains(&(Some(4), Kind::NewError)));
        assert!(f.iter().any(|f| f.kind == Kind::Skipped && f.text.contains("CreateMutexW")), "{f:?}");
        // SetEvent was called: not skipped. Functions after the last one Poly reached are not either.
        assert!(!f.iter().any(|f| f.text.contains("SetEvent")));
    }

    #[test]
    fn the_kits_scope_limits_what_counts_as_skipped() {
        let poly = parse_poly(&poly_text());
        let (win, objects) = parse_windows(&win_doc(), "browser").unwrap();
        let kit = json!({"proxies": [{"dll": "kernel32.dll", "functions": [{"name": "CreateFileW"}]}]});
        let scope = kit_scope(&kit);
        let f = compare(&poly, &win, &objects, Some(&scope));
        assert!(!f.iter().any(|f| f.kind == Kind::Skipped), "{f:?}");
    }

    #[test]
    fn names_are_normalised_like_the_reduction() {
        assert_eq!(normalise("C:\\Program Files\\Chrome\\chrome_elf.dll"), "<app>\\chrome_elf.dll");
        assert_eq!(normalise("C:\\poly-test\\profile\\Default\\History"), "<profile>\\Default\\History");
        assert_eq!(normalise("{0F5E1C52-1234-4B6A-8C3D-0123456789AB}"), "{<guid>}");
        assert_eq!(normalise("\\\\.\\pipe\\mojo.1234.5678.9"), "\\\\.\\pipe\\mojo.<n>.<n>.9");
        assert_eq!(normalise("S-1-5-21-0-0-0-1000"), "S-1-5-21-<domain>");
        assert_eq!(normalise("key\\DisplayVersion"), "key\\DisplayVersion");
    }

    #[test]
    fn a_clean_trace_reports_no_difference() {
        let poly = parse_poly(r#"{"seq":0,"t":0,"dll":"kernel32.dll","fn":"GetVersionExW","args":[],"ret":1}"#);
        let (win, objects) = parse_windows(&win_doc(), "browser").unwrap();
        let scope: BTreeSet<_> = [("kernel32.dll".to_string(), "GetVersionExW".to_string())].into();
        let f = compare(&poly, &win, &objects, Some(&scope));
        assert!(f.is_empty(), "{f:?}");
        assert!(report(&f, 1, 1).contains("no difference"));
    }
}
