//! Spec seeds from Wine's conformance tests (ADR 0097): which test
//! functions of `dlls/<dll>/tests/*.c` call which Windows exports, so a
//! spec agent knows where Windows behaviour for an entry is tested.
//!
//! Wine's tests are LGPL and an oracle for spec agents only (CLEAN_ROOM.md).
//! Only names leave the checkout: the test file's path below `dlls/`, the
//! name of a function in it that checks something (`ok(...)`), and the
//! names of the coverage map's exports it calls. No code, comments,
//! strings, arguments or expectations are kept, and nothing is copied.
//!
//! The reading is lexical (comments, literals and preprocessor lines are
//! skipped, functions found by their `name(...) {` shape), which is all a
//! list of names needs; a call through Wine's usual `pName` function
//! pointer counts as a call of `Name`.

use crate::model::{Coverage, Layer, Os};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The seed source's key in `Coverage::seeds` and the seeds' prefix.
pub const WINE: &str = "wine";
pub const WINE_DESCRIPTION: &str = "Wine conformance tests (wine-mirror/wine dlls/*/tests/*.c): the test functions \
     that call an export, by name only; spec agents' oracle, never copied (CLEAN_ROOM.md, ADR 0097)";
/// At most this many seeds per entry, best first.
pub const MAX_SEEDS: usize = 8;

/// Components every test may call besides its own DLL's: the base system.
const BASE: &[&str] = &["ntdll.dll", "kernel32.dll", "kernelbase.dll"];

/// One function defined in a test file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// It checks something itself: `ok(` or an `ok_*(` macro.
    pub checks: bool,
    /// The names it calls (`name(`), with how often.
    pub calls: BTreeMap<String, u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok<'a> {
    Ident(&'a str),
    Punct(u8),
}

const KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "return",
    "sizeof",
    "defined",
    "goto",
    "break",
    "continue",
    "__typeof__",
    "typeof",
    "_Alignof",
    "__alignof__",
    "__attribute__",
    "__declspec",
    "__asm__",
    "asm",
    "volatile",
];

fn skip_block_comment(b: &[u8], mut i: usize) -> usize {
    i += 2;
    while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
        i += 1;
    }
    (i + 2).min(b.len())
}

/// Past a string or character literal starting at `i`.
fn skip_literal(b: &[u8], mut i: usize) -> usize {
    let quote = b[i];
    i += 1;
    while i < b.len() && b[i] != quote && b[i] != b'\n' {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    (i + 1).min(b.len())
}

/// The identifiers and punctuation of C source, without comments,
/// literals, numbers and preprocessor lines. Of a conditional only the
/// first branch is kept (`#if A ... #else ... #endif` reads as `A`'s), so
/// braces opened in both branches do not unbalance a function.
fn tokens(src: &str) -> Vec<Tok<'_>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut line_start) = (0, true);
    // > 0: inside a later branch, this many conditionals deep.
    let mut skip = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(b, i);
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'#' && line_start {
            // A directive, continuation lines and comments included.
            let word: String =
                src[i + 1..].trim_start_matches([' ', '\t']).chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            match word.as_str() {
                "if" | "ifdef" | "ifndef" if skip > 0 => skip += 1,
                "else" | "elif" | "elifdef" | "elifndef" if skip == 0 => skip = 1,
                "endif" if skip > 0 => skip -= 1,
                _ => {}
            }
            while i < b.len() && b[i] != b'\n' {
                if b[i] == b'\\' && b.get(i + 1) == Some(&b'\n') {
                    i += 2;
                } else if b[i] == b'\\' && b.get(i + 1) == Some(&b'\r') && b.get(i + 2) == Some(&b'\n') {
                    i += 3;
                } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    i = skip_block_comment(b, i);
                } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
        } else {
            line_start = false;
            let tok = if c == b'"' || c == b'\'' {
                i = skip_literal(b, i);
                Some(Tok::Punct(b'"'))
            } else if c.is_ascii_alphabetic() || c == b'_' {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                Some(Tok::Ident(&src[s..i]))
            } else if c.is_ascii_digit() {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                    i += 1;
                }
                None
            } else {
                i += 1;
                Some(Tok::Punct(c))
            };
            if let (Some(t), 0) = (tok, skip) {
                out.push(t);
            }
        }
    }
    out
}

/// `... name ( params ) {`: the name of the function whose body starts
/// after `before`, if `before` ends like a definition.
fn definition_name<'a>(before: &[Tok<'a>]) -> Option<&'a str> {
    let mut j = before.len().checked_sub(1)?;
    if before[j] != Tok::Punct(b')') {
        return None;
    }
    let mut level = 0;
    loop {
        match before[j] {
            Tok::Punct(b')') => level += 1,
            Tok::Punct(b'(') => {
                level -= 1;
                if level == 0 {
                    break;
                }
            }
            Tok::Punct(b';' | b'{' | b'}' | b'=') => return None,
            _ => {}
        }
        j = j.checked_sub(1)?;
    }
    match before.get(j.checked_sub(1)?) {
        Some(&Tok::Ident(n)) if !KEYWORDS.contains(&n) => Some(n),
        _ => None,
    }
}

/// The functions defined in C source, in order.
pub fn functions(src: &str) -> Vec<Function> {
    let toks = tokens(src);
    let mut out = Vec::new();
    let (mut depth, mut i) = (0usize, 0);
    while i < toks.len() {
        match toks[i] {
            Tok::Punct(b'{') if depth == 0 => {
                if let Some(name) = definition_name(&toks[..i]) {
                    let mut f = Function { name: name.to_string(), ..Function::default() };
                    let mut level = 0usize;
                    while i < toks.len() {
                        match toks[i] {
                            Tok::Punct(b'{') => level += 1,
                            Tok::Punct(b'}') => {
                                level -= 1;
                                if level == 0 {
                                    break;
                                }
                            }
                            Tok::Ident(n) if toks.get(i + 1) == Some(&Tok::Punct(b'(')) && !KEYWORDS.contains(&n) => {
                                if n == "ok" || n.starts_with("ok_") {
                                    f.checks = true;
                                } else {
                                    *f.calls.entry(n.to_string()).or_default() += 1;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    out.push(f);
                } else {
                    depth = 1;
                }
            }
            Tok::Punct(b'{') => depth += 1,
            Tok::Punct(b'}') => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    out
}

/// The export names a called name can stand for: itself, and for Wine's
/// function pointers `pName` / `p_name` the name without the prefix.
fn candidates(called: &str) -> Vec<&str> {
    let mut v = vec![called];
    if let Some(rest) = called.strip_prefix('p') {
        if rest.starts_with(|c: char| c.is_ascii_uppercase() || c == '_') {
            v.push(rest);
        }
        if let Some(r) = rest.strip_prefix('_').filter(|r| !r.is_empty()) {
            v.push(r);
        }
    }
    v
}

fn squash(s: &str) -> String {
    s.chars().filter(|&c| c != '_').map(|c| c.to_ascii_lowercase()).collect()
}

/// How closely a test function's name names `symbol`: 0 it is the
/// symbol's test (`test_CreateFileW`, `test_CreateFile` for
/// `CreateFileW`), 1 it is about the symbol (`test_CreateFile_share`),
/// 2 neither, or it is not a test but a helper (`create_file`).
pub fn name_match(function: &str, symbol: &str) -> u8 {
    let Some(core) = function.strip_prefix("test_").or_else(|| function.strip_prefix("test")) else {
        return 2;
    };
    let (core, sym) = (squash(core), squash(symbol));
    let b = symbol.as_bytes();
    let narrow = b.len() > 2 && matches!(b[b.len() - 1], b'A' | b'W') && b[b.len() - 2].is_ascii_lowercase();
    let stem = if narrow { &sym[..sym.len() - 1] } else { &sym[..] };
    if core == sym || core == stem {
        0
    } else if stem.len() >= 4 && core.contains(stem) {
        1
    } else {
        2
    }
}

/// The Wine `dlls/` directories whose tests are a component's own.
fn home_dirs(layer: Layer, component: &str) -> Vec<String> {
    match (layer, component) {
        (Layer::Syscall, "ntoskrnl") => vec!["ntdll".into()],
        (Layer::Syscall, "win32k") => vec!["win32u".into()],
        (Layer::Syscall, c) => vec![c.into()],
        (Layer::Api, "kernelbase.dll") => vec!["kernelbase".into(), "kernel32".into()],
        (Layer::Api, c) => vec![c.strip_suffix(".dll").unwrap_or(c).into()],
    }
}

/// The user-mode DLL whose exports are a system-call table's stubs.
fn syscall_table(dll: &str) -> Option<&'static str> {
    match dll {
        "ntdll.dll" => Some("ntoskrnl"),
        "win32u.dll" => Some("win32k"),
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub files: usize,
    pub functions: usize,
    pub checking: usize,
    /// (entry, test function) pairs found, before the per-entry limit.
    pub pairs: usize,
}

/// One test file: `<dll>/tests/<file>.c` below `dlls/` and its source.
pub struct TestFile {
    pub dll: String,
    pub path: String,
    pub source: String,
}

/// Every `dlls/<dll>/tests/*.c` of a Wine checkout, sorted.
pub fn wine_test_files(wine: &Path) -> Result<Vec<TestFile>, String> {
    let dlls = wine.join("dlls");
    let read = std::fs::read_dir(&dlls).map_err(|e| format!("{}: {e}", dlls.display()))?;
    let mut dirs: Vec<String> =
        read.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    dirs.sort();
    let mut out = Vec::new();
    for dll in dirs {
        let tests = dlls.join(&dll).join("tests");
        let Ok(read) = std::fs::read_dir(&tests) else { continue };
        let mut files: Vec<String> = read
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".c"))
            .collect();
        files.sort();
        for f in files {
            let bytes = std::fs::read(tests.join(&f)).map_err(|e| format!("{}: {e}", tests.join(&f).display()))?;
            out.push(TestFile {
                path: format!("{dll}/tests/{f}"),
                dll: dll.to_ascii_lowercase(),
                source: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
    }
    Ok(out)
}

/// The seeds of every Windows entry of `c` that the test files call:
/// entry id to `<path>:<function>`, best first, at most [`MAX_SEEDS`].
///
/// A call seeds the entries of that name in the test's own DLL and in the
/// base system (`ntdll`, `kernel32`, `kernelbase`; with `ntdll`'s, the
/// system-call entry of the same name). Ranked by how closely the test
/// function's name names the export, then whether the test is the
/// entry's DLL's own, then whether it is a test (`test*`) rather than a
/// checking helper, then how few exports the function calls (focus),
/// then how often it calls this one.
pub fn seeds(c: &Coverage, files: &[TestFile]) -> (BTreeMap<String, Vec<String>>, Stats) {
    // symbol -> (component key, entry index)
    let mut by_symbol: BTreeMap<&str, Vec<(Layer, &str, usize)>> = BTreeMap::new();
    let mut api_components = BTreeSet::new();
    for (i, e) in c.entries.iter().enumerate() {
        if e.os != Os::Windows || e.symbol.starts_with('#') {
            continue;
        }
        by_symbol.entry(&e.symbol).or_default().push((e.layer, &e.component, i));
        if e.layer == Layer::Api {
            api_components.insert(e.component.as_str());
        }
    }
    let mut stats = Stats::default();
    // entry index -> (rank key, seed)
    type Key = (u8, u8, bool, usize, std::cmp::Reverse<u32>);
    let mut hits: BTreeMap<usize, Vec<(Key, String)>> = BTreeMap::new();
    for file in files {
        stats.files += 1;
        let home = if file.dll.contains('.') { file.dll.clone() } else { format!("{}.dll", file.dll) };
        let home = api_components.contains(home.as_str()).then_some(home);
        let funcs = functions(&file.source);
        let local: BTreeSet<&str> = funcs.iter().map(|f| f.name.as_str()).collect();
        stats.functions += funcs.len();
        for f in funcs.iter().filter(|f| f.checks) {
            stats.checking += 1;
            // entry index -> (symbol, times called)
            let mut called: BTreeMap<usize, (&str, u32)> = BTreeMap::new();
            for (name, &n) in &f.calls {
                for sym in candidates(name) {
                    if local.contains(sym) {
                        continue;
                    }
                    let Some(list) = by_symbol.get(sym) else { continue };
                    let allowed = |layer: Layer, comp: &str| {
                        layer == Layer::Api && (home.as_deref() == Some(comp) || BASE.contains(&comp))
                    };
                    let tables: Vec<&str> = list
                        .iter()
                        .filter(|(l, comp, _)| allowed(*l, comp))
                        .filter_map(|(_, comp, _)| syscall_table(comp))
                        .collect();
                    for &(layer, comp, i) in list {
                        if allowed(layer, comp) || (layer == Layer::Syscall && tables.contains(&comp)) {
                            let slot = called.entry(i).or_insert((sym, 0));
                            slot.1 += n;
                        }
                    }
                }
            }
            let focus = called.values().map(|(s, _)| *s).collect::<BTreeSet<_>>().len();
            for (i, (sym, n)) in called {
                let e = &c.entries[i];
                let own = u8::from(!home_dirs(e.layer, &e.component).contains(&file.dll));
                let helper = !f.name.starts_with("test");
                let key = (name_match(&f.name, sym), own, helper, focus, std::cmp::Reverse(n));
                hits.entry(i).or_default().push((key, format!("{}:{}", file.path, f.name)));
                stats.pairs += 1;
            }
        }
    }
    let mut out = BTreeMap::new();
    for (i, mut list) in hits {
        list.sort();
        let mut seen = BTreeSet::new();
        let seeds: Vec<String> =
            list.into_iter().map(|(_, s)| s).filter(|s| seen.insert(s.clone())).take(MAX_SEEDS).collect();
        out.insert(c.entries[i].id.clone(), seeds);
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entry, Source};

    // A made-up test file in the shape of a conformance test (not from Wine).
    const SAMPLE: &str = r#"
#include <windows.h>
#define check_thing(a) \
    ok((a), "thing { not a body\n")
static NTSTATUS (WINAPI *pNtOpenFoo)(HANDLE *);
static const struct { int a; } table[] = { { 1 }, { 2 } };

/* static void test_commented(void) { ok(1, "x"); OpenFooW(0); } */
static void helper(HANDLE h)
{
    CloseFoo(h);
}

static void test_OpenFoo(void)
{
    HANDLE h = OpenFooW(L"name(");  // OpenFooA( in a comment
    ok(h != NULL, "OpenFooW failed\n");
    if (h) helper(h);
    CloseFoo(h);
    CloseFoo(h);
}

static DWORD CALLBACK thread_proc(void *arg)
{
#ifdef SOMETHING
    if (arg) {
#else
    if (!arg) {
#endif
        pNtOpenFoo(arg);
    }
    ok_status(0, 0);
    return 0;
}

static void test_misc(void)
{
    OpenFooW(0);
    CloseFoo(0);
    WaitFoo(0);
    ok(1, "x\n");
}

START_TEST(foo)
{
    test_OpenFoo();
    helper(0);
}
"#;

    #[test]
    fn finds_functions_checks_and_calls() {
        let f = functions(SAMPLE);
        let names: Vec<&str> = f.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["helper", "test_OpenFoo", "thread_proc", "test_misc", "START_TEST"]);
        assert!(!f[0].checks && f[1].checks && f[2].checks && f[3].checks && !f[4].checks);
        let calls: Vec<(&str, u32)> = f[1].calls.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(calls, [("CloseFoo", 2), ("OpenFooW", 1), ("helper", 1)]);
        assert_eq!(f[2].calls.keys().collect::<Vec<_>>(), ["pNtOpenFoo"]);
    }

    #[test]
    fn names_and_candidates() {
        assert_eq!(name_match("test_CreateFileW", "CreateFileW"), 0);
        assert_eq!(name_match("test_CreateFile", "CreateFileW"), 0);
        assert_eq!(name_match("test_createfile", "CreateFileA"), 0);
        assert_eq!(name_match("test_CreateFile_share", "CreateFileW"), 1);
        assert_eq!(name_match("test_ReadFile", "CreateFileW"), 2);
        assert_eq!(name_match("create_file", "CreateFileW"), 2);
        assert_eq!(name_match("test_NtOpenKey", "NtOpenKey"), 0);
        assert_eq!(name_match("testGetVersion", "GetVersion"), 0);
        assert_eq!(candidates("pNtOpenFoo"), ["pNtOpenFoo", "NtOpenFoo"]);
        assert_eq!(candidates("p_wcsicmp"), ["p_wcsicmp", "_wcsicmp", "wcsicmp"]);
        assert_eq!(candidates("printf"), ["printf"]);
    }

    #[test]
    fn seeds_rank_and_scope() {
        let mut c = Coverage::default();
        c.sources.insert("t".into(), Source { description: "t".into(), revision: "1".into() });
        let mut add = |layer, comp: &str, sym: &str| c.merge(vec![Entry::new(Os::Windows, layer, comp, sym, "t")]);
        add(Layer::Api, "foo.dll", "OpenFooW");
        add(Layer::Api, "foo.dll", "CloseFoo");
        add(Layer::Api, "kernel32.dll", "WaitFoo");
        add(Layer::Api, "ntdll.dll", "NtOpenFoo");
        add(Layer::Syscall, "ntoskrnl", "NtOpenFoo");
        add(Layer::Api, "other.dll", "WaitFoo");
        add(Layer::Api, "foo.dll", "helper");
        let files = vec![
            TestFile { dll: "foo".into(), path: "foo/tests/foo.c".into(), source: SAMPLE.into() },
            TestFile { dll: "bar".into(), path: "bar/tests/b.c".into(), source: SAMPLE.into() },
        ];
        let (s, stats) = seeds(&c, &files);
        assert_eq!((stats.files, stats.functions, stats.checking), (2, 10, 6));
        let get = |id: &str| s.get(id).cloned().unwrap_or_default();
        // The test named after it first, its own DLL's before others.
        assert_eq!(get("windows/api/foo.dll/OpenFooW"), ["foo/tests/foo.c:test_OpenFoo", "foo/tests/foo.c:test_misc"]);
        // bar's tests do not reach foo.dll; a defined function is not an export.
        assert_eq!(get("windows/api/foo.dll/CloseFoo"), ["foo/tests/foo.c:test_OpenFoo", "foo/tests/foo.c:test_misc"]);
        assert!(get("windows/api/foo.dll/helper").is_empty());
        // The base system is reached from every test; other DLLs are not.
        assert_eq!(get("windows/api/kernel32.dll/WaitFoo"), ["bar/tests/b.c:test_misc", "foo/tests/foo.c:test_misc"]);
        assert!(get("windows/api/other.dll/WaitFoo").is_empty());
        // Function pointers count; ntdll's system calls come along.
        assert_eq!(
            get("windows/api/ntdll.dll/NtOpenFoo"),
            ["bar/tests/b.c:thread_proc", "foo/tests/foo.c:thread_proc"]
        );
        assert_eq!(get("windows/syscall/ntoskrnl/NtOpenFoo"), get("windows/api/ntdll.dll/NtOpenFoo"));
    }
}
