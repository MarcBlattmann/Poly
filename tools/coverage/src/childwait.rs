//! A test that reads the exit code of a process or thread it started must wait for it first (`coverage check`, warnings).
//!
//! `GetExitCodeProcess` answers 259 (`STILL_ACTIVE`) until the process has ended, so a read that comes before
//! any wait gives a result that depends on timing: on Windows the child is usually quicker than the test, on a
//! loaded CI runner it is not (`wpm_rules_c`, `qij_basic`, `pbi_peb_c`, `vpe_acg_c` all failed that way, with 259
//! or with a timeout value). This check finds, per function of a Windows test source, a `GetExitCodeProcess`,
//! `GetExitCodeThread` or `exit_code_of_process/thread` call with no earlier call of something named `wait*` or
//! `finish*` (or `.finish()`, `run_child(`) in the same function. It is a heuristic: a read of a suspended or
//! running process on purpose, or after a wait made in the caller, is legitimate; those are listed in
//! `tools/coverage/childwait-baseline.txt` (`path:function`, one per line) after someone looked at them, and only
//! hits that are not there are reported. It does not look at how long the wait is.
//!
//! Sources: `specs/windows/**/*.tests/*.rs`, `tests/windows/*.rs` and, when the held-out store is present, its
//! test files (named `heldout/<dll>/<entry>/<file>` in the baseline); the held-out files are read by this local
//! check only and nothing of them is printed but the path and the function name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One finding: where, in which function, on which line.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Hit {
    pub path: String,
    pub function: String,
    pub line: usize,
}

/// The identifiers that count as "waits for the process" in the text before a read.
fn has_wait(before: &str) -> bool {
    let b = before.as_bytes();
    let lower = before.to_ascii_lowercase();
    let l = lower.as_bytes();
    for key in ["wait", "finish"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(key) {
            let at = from + i;
            from = at + key.len();
            // the start of an identifier ("wait_exit", ".finish", "WaitForSingleObject"): not the middle of a word like "awaiting"
            let starts = at == 0 || !(l[at - 1].is_ascii_alphanumeric());
            if !starts && !(at >= 1 && l[at - 1] == b'_') {
                continue;
            }
            // skip the rest of the identifier, spaces, then a call
            let mut j = from;
            while j < l.len() && (l[j].is_ascii_alphanumeric() || l[j] == b'_') {
                j += 1;
            }
            while j < l.len() && (l[j] == b' ' || l[j] == b'\t') {
                j += 1;
            }
            if j < b.len() && b[j] == b'(' {
                return true;
            }
        }
    }
    before.contains("run_child(")
}

/// The positions of the exit-code reads in `body` (outside the name of a function that is being declared).
fn reads(body: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for key in ["GetExitCodeProcess(", "GetExitCodeThread(", "exit_code_of_process(", "exit_code_of_thread("] {
        let mut from = 0;
        while let Some(i) = body[from..].find(key) {
            let at = from + i;
            from = at + key.len();
            // a declaration (`fn exit_code_of_process(`) is not a call
            if body[..at].trim_end().ends_with("fn") {
                continue;
            }
            out.push(at);
        }
    }
    out.sort_unstable();
    out
}

/// The text of the function whose `fn` keyword starts at `pos`, or `None` for a declaration without a body.
fn body_of(text: &str, pos: usize) -> Option<&str> {
    let b = text.as_bytes();
    let mut paren = 0i32;
    let mut i = pos;
    while i < b.len() {
        match b[i] {
            b'(' => paren += 1,
            b')' => paren -= 1,
            b';' if paren == 0 => return None,
            b'{' if paren == 0 => {
                let mut depth = 0i32;
                let mut j = i;
                while j < b.len() {
                    match b[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(&text[pos..=j]);
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                return Some(&text[pos..]);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The functions of `text` as `(name, byte position of its `fn`)`.
fn functions(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let mut rest = t;
        for prefix in ["pub ", "unsafe ", "extern \"system\" "] {
            while let Some(r) = rest.strip_prefix(prefix) {
                rest = r;
            }
        }
        if let Some(r) = rest.strip_prefix("fn ") {
            let name: String = r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if !name.is_empty() {
                let start = offset + (line.len() - t.len()) + (t.len() - rest.len());
                out.push((name, start));
            }
        }
        offset += line.len();
    }
    out
}

/// The reads in `text` that have no wait before them in their function.
pub fn scan_text(path: &str, text: &str) -> Vec<Hit> {
    if !text.contains("GetExitCode") && !text.contains("exit_code_of_") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, pos) in functions(text) {
        // the helper that is only the read (`fn exit_code_of_process`): its callers are checked instead
        if name.starts_with("exit_code_of_") {
            continue;
        }
        let Some(body) = body_of(text, pos) else { continue };
        for at in reads(body) {
            if !has_wait(&body[..at]) {
                let line = text[..pos].matches('\n').count() + 1 + body[..at].matches('\n').count();
                out.push(Hit { path: path.to_string(), function: name.clone(), line });
            }
        }
    }
    out
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 6 {
                rs_files(&p, out, depth + 1);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `path:function` lines of the reviewed baseline.
pub fn baseline(text: &str) -> BTreeSet<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()
}

/// The hits of the repository at `root` (and of the held-out store, when there is one) that are not in the baseline.
pub fn check(root: &Path, store: Option<&Path>) -> Vec<Hit> {
    let known = std::fs::read_to_string(root.join("tools/coverage/childwait-baseline.txt"))
        .map(|t| baseline(&t))
        .unwrap_or_default();
    let mut files = Vec::new();
    rs_files(&root.join("specs/windows"), &mut files, 0);
    if let Ok(rd) = std::fs::read_dir(root.join("tests/windows")) {
        files
            .extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "rs")));
    }
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|p| (p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/"), p))
        .collect();
    if let Some(store) = store {
        let mut held = Vec::new();
        rs_files(&store.join("windows/api"), &mut held, 0);
        for p in held {
            let rel = p.strip_prefix(store.join("windows/api")).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            named.push((format!("heldout/{rel}"), p));
        }
    }
    let mut out = Vec::new();
    for (name, path) in named {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for h in scan_text(&name, &text) {
            if !known.contains(&format!("{}:{}", h.path, h.function)) {
                out.push(h);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_before_any_wait_is_a_hit() {
        let t = "fn test() {\n    let c = start();\n    unsafe { GetExitCodeProcess(c, &mut code) };\n}\n";
        let h = scan_text("a.rs", t);
        assert_eq!(h, vec![Hit { path: "a.rs".into(), function: "test".into(), line: 3 }]);
    }

    #[test]
    fn a_wait_before_the_read_is_fine() {
        let t = "fn test() {\n    unsafe { WaitForSingleObject(p, 10_000); GetExitCodeProcess(p, &mut c) };\n}\n";
        assert!(scan_text("a.rs", t).is_empty());
        let t2 = "fn test() {\n    wait_exit(p, 5000);\n    let c = exit_code_of_process(p);\n}\n";
        assert!(scan_text("a.rs", t2).is_empty());
        let t3 = "fn run() -> u32 {\n    let c = start();\n    let code = c.finish();\n    unsafe { GetExitCodeThread(t, &mut x) };\n    code\n}\n";
        assert!(scan_text("a.rs", t3).is_empty());
    }

    #[test]
    fn a_wait_after_the_read_does_not_count() {
        let t = "fn test() {\n    GetExitCodeProcess(p, &mut c);\n    WaitForSingleObject(p, 5);\n}\n";
        assert_eq!(scan_text("a.rs", t).len(), 1);
    }

    #[test]
    fn declarations_and_the_read_helper_are_not_hits() {
        let t = "unsafe extern \"system\" {\n    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;\n}\nfn exit_code_of_process(h: HANDLE) -> i64 {\n    let ok = unsafe { GetExitCodeProcess(h, &mut c) };\n    c as i64\n}\n";
        assert!(scan_text("a.rs", t).is_empty());
    }

    #[test]
    fn the_word_wait_inside_another_word_is_not_a_wait() {
        let t = "fn test() {\n    let awaiting = 1;\n    GetExitCodeProcess(p, &mut c);\n}\n";
        assert_eq!(scan_text("a.rs", t).len(), 1);
    }

    #[test]
    fn the_baseline_hides_reviewed_hits() {
        let b = baseline("# comment\nspecs/windows/x.dll/E.tests/a.rs:test\n\n");
        assert!(b.contains("specs/windows/x.dll/E.tests/a.rs:test"));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn check_reads_the_tree_and_the_baseline() {
        let dir = std::env::temp_dir().join(format!("poly-childwait-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = dir.join("specs/windows/k.dll/E.tests");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::create_dir_all(dir.join("tools/coverage")).unwrap();
        std::fs::write(w.join("bad.rs"), "fn test() {\n    GetExitCodeProcess(p, &mut c);\n}\n").unwrap();
        std::fs::write(w.join("known.rs"), "fn test() {\n    GetExitCodeProcess(p, &mut c);\n}\n").unwrap();
        std::fs::write(
            w.join("good.rs"),
            "fn test() {\n    WaitForSingleObject(p, 9);\n    GetExitCodeProcess(p, &mut c);\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tools/coverage/childwait-baseline.txt"),
            "specs/windows/k.dll/E.tests/known.rs:test\n",
        )
        .unwrap();
        let hits = check(&dir, None);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, "specs/windows/k.dll/E.tests/bad.rs");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
