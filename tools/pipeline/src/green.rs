//! Green entries keep passing: the coverage entries marked `green` that have
//! visible spec tests (`specs/<os>/<component>/<symbol>.tests/`), which
//! `pipeline polycheck` runs on Poly next to the corpus, and the check of a
//! polycheck run against them and against `tests/linux/poly-pass.txt`.
//!
//! Held-out tests are not in the repository (CLEAN_ROOM.md), so CI can only
//! keep the visible half green; the held-out half is checked by the runner
//! before an entry is marked green (`pipeline diff` with both directories).

use crate::diff::{Summary, Verdict};
use crate::store::EntryId;
use crate::util::{self, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct CoverageEntry {
    symbol: String,
    #[serde(default)]
    status: Option<String>,
}

/// A green entry and its visible tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GreenEntry {
    pub id: String,
    /// Relative to the repository root.
    pub tests_dir: PathBuf,
    /// Test program names (file stems of the `.c` files), sorted.
    pub tests: Vec<String>,
}

/// The file stems of the files in `dir` with one of `extensions`, sorted.
fn stems(dir: &Path, extensions: &[&str]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|x| x.to_str()).is_some_and(|x| extensions.contains(&x)) && path.is_file() {
            out.push(path.file_stem().unwrap().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

fn c_stems(dir: &Path) -> Result<Vec<String>> {
    stems(dir, &["c"])
}

fn subdirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let Ok(read) = std::fs::read_dir(dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for e in read {
        let path = e.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Every green entry with visible tests. Only the coverage files of
/// components that have a `specs/` directory are read (the Windows and
/// macOS API files are large).
pub fn entries(repo: &Path) -> Result<Vec<GreenEntry>> {
    entries_with(repo, c_stems)
}

/// A green Windows entry (ADR 0075, 0098) with visible tests and their
/// recorded Windows results (`<symbol>.tests/windows.json`), which
/// `pipeline win polycheck` compares Poly with (ADR 0099).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsEntry {
    pub entry: GreenEntry,
    /// The recorded results, relative to the repository root.
    pub reference: PathBuf,
}

/// Every green Windows entry whose tests directory has `.rs`/`.cpp` tests
/// and a `windows.json`.
pub fn windows_entries(repo: &Path) -> Result<Vec<WindowsEntry>> {
    let all = entries_with(repo, |dir| stems(dir, crate::windows::TEST_EXTENSIONS))?;
    Ok(all
        .into_iter()
        .filter(|g| g.id.starts_with("windows/"))
        .map(|g| WindowsEntry { reference: g.tests_dir.join("windows.json"), entry: g })
        .filter(|w| repo.join(&w.reference).is_file())
        .collect())
}

fn entries_with(repo: &Path, tests_of: impl Fn(&Path) -> Result<Vec<String>>) -> Result<Vec<GreenEntry>> {
    let mut out = Vec::new();
    for os_dir in subdirs(&repo.join("specs"))? {
        let os = name(&os_dir);
        for component_dir in subdirs(&os_dir)? {
            let component = name(&component_dir);
            for layer_dir in subdirs(&repo.join("coverage").join(&os))? {
                let file = layer_dir.join(format!("{component}.json"));
                if !file.is_file() {
                    continue;
                }
                let layer = name(&layer_dir);
                let list: Vec<CoverageEntry> = util::read_json(&file)?;
                for e in list.into_iter().filter(|e| e.status.as_deref() == Some("green")) {
                    let id = EntryId::parse(&format!("{os}/{layer}/{component}/{}", e.symbol))?;
                    let tests_dir = id.tests_dir();
                    if !repo.join(&tests_dir).is_dir() {
                        continue;
                    }
                    let tests = tests_of(&repo.join(&tests_dir))?;
                    if !tests.is_empty() {
                        out.push(GreenEntry { id: id.id(), tests_dir, tests });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// What a polycheck run says.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// Tests of the pass list that did not pass.
    pub corpus_regressions: Vec<String>,
    /// Green entries with visible tests that did not pass: entry id, tests.
    pub entry_regressions: BTreeMap<String, Vec<String>>,
    /// Corpus tests that pass but are not on the pass list.
    pub newly_passing: Vec<String>,
}

impl Check {
    /// One line per failure; empty when the run is clean.
    pub fn problems(&self, pass_list: &str) -> Vec<String> {
        let mut out = Vec::new();
        if !self.corpus_regressions.is_empty() {
            out.push(format!(
                "{} no longer pass{} (listed in {pass_list})",
                self.corpus_regressions.join(", "),
                if self.corpus_regressions.len() == 1 { "es" } else { "" }
            ));
        }
        for (id, tests) in &self.entry_regressions {
            let s = if tests.len() == 1 { "" } else { "s" };
            out.push(format!(
                "green entry {id}: visible test{s} {} fail{}",
                tests.join(", "),
                if s.is_empty() { "s" } else { "" }
            ));
        }
        out
    }
}

/// Check a run of the corpus and the green entries' visible tests.
pub fn check(summary: &Summary, pass_list: &[String], green: &[GreenEntry]) -> Check {
    let passed = |name: &str| summary.tests.iter().any(|t| t.test == name && t.verdict == Verdict::Pass);
    let entry_test = |name: &str| green.iter().any(|g| g.tests.iter().any(|t| t == name));
    let mut c =
        Check { corpus_regressions: pass_list.iter().filter(|t| !passed(t)).cloned().collect(), ..Check::default() };
    for g in green {
        let failed: Vec<String> = g.tests.iter().filter(|t| !passed(t)).cloned().collect();
        if !failed.is_empty() {
            c.entry_regressions.insert(g.id.clone(), failed);
        }
    }
    c.newly_passing = summary
        .tests
        .iter()
        .filter(|t| t.verdict == Verdict::Pass && !pass_list.contains(&t.test) && !entry_test(&t.test))
        .map(|t| t.test.clone())
        .collect();
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{SyscallMode, TestResult};

    fn summary(results: &[(&str, Verdict)]) -> Summary {
        let mut s = Summary::new("now".into(), "poly", "linux".into(), "ptrace", SyscallMode::Exact);
        for (name, v) in results {
            let mut r = TestResult::not_run(name, *v, String::new());
            r.reason = None;
            s.tests.push(r);
        }
        s
    }

    fn green(id: &str, tests: &[&str]) -> GreenEntry {
        GreenEntry {
            id: id.into(),
            tests_dir: PathBuf::from("specs/x"),
            tests: tests.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn a_failing_visible_test_of_a_green_entry_fails_the_check() {
        let s = summary(&[
            ("hello", Verdict::Pass),
            ("brk", Verdict::Pass),
            ("uname", Verdict::Pass),
            ("a_close_many", Verdict::Pass),
            ("a_close_shared", Verdict::Fail),
            ("a_dup3_basic", Verdict::Pass),
        ]);
        let list = vec!["hello".to_string(), "brk".to_string()];
        let g = vec![
            green("linux/syscall/x86_64/close", &["a_close_many", "a_close_shared"]),
            green("linux/syscall/x86_64/dup3", &["a_dup3_basic"]),
        ];
        let c = check(&s, &list, &g);
        assert_eq!(
            c.problems("poly-pass.txt"),
            ["green entry linux/syscall/x86_64/close: visible test a_close_shared fails"]
        );
        assert!(c.corpus_regressions.is_empty());
        assert_eq!(c.entry_regressions.len(), 1);
        assert_eq!(c.entry_regressions["linux/syscall/x86_64/close"], ["a_close_shared"]);
        // Entry tests are never proposed for the corpus pass list.
        assert_eq!(c.newly_passing, ["uname"]);
    }

    #[test]
    fn unsupported_and_missing_tests_are_not_passes() {
        let s = summary(&[("hello", Verdict::Unsupported), ("a_x", Verdict::Error)]);
        let c = check(&s, &["hello".to_string()], &[green("linux/syscall/x86_64/x", &["a_x", "a_y"])]);
        assert_eq!(c.corpus_regressions, ["hello"]);
        assert_eq!(c.entry_regressions["linux/syscall/x86_64/x"], ["a_x", "a_y"]);
        assert_eq!(
            c.problems("poly-pass.txt"),
            [
                "hello no longer passes (listed in poly-pass.txt)",
                "green entry linux/syscall/x86_64/x: visible tests a_x, a_y fail"
            ]
        );
        let s = summary(&[("hello", Verdict::Pass), ("a_x", Verdict::Pass), ("a_y", Verdict::Pass)]);
        let c = check(&s, &["hello".to_string()], &[green("linux/syscall/x86_64/x", &["a_x", "a_y"])]);
        assert!(c.problems("poly-pass.txt").is_empty());
    }

    /// The real repository: every green Windows entry has recorded results
    /// for each of its visible tests; the C-test entries are not Windows
    /// ones.
    #[test]
    fn finds_green_windows_entries_with_their_results() {
        let repo = crate::repo_root();
        for w in windows_entries(&repo).unwrap() {
            assert!(w.entry.id.starts_with("windows/"), "{w:?}");
            let results = crate::windows::RunFile::load(&repo.join(&w.reference)).unwrap();
            for t in &w.entry.tests {
                assert!(results.test(t).is_some(), "{}: no recorded result for {t}", w.entry.id);
            }
        }
        assert!(entries(&repo).unwrap().iter().all(|g| !g.id.starts_with("windows/")));
    }

    /// The real repository: every green entry found has its tests directory
    /// under `specs/`, and every test there is named.
    #[test]
    fn finds_green_entries_in_the_repository() {
        let repo = crate::repo_root();
        for g in entries(&repo).unwrap() {
            assert!(g.tests_dir.starts_with("specs"), "{g:?}");
            assert!(!g.tests.is_empty(), "{g:?}");
            assert!(repo.join(&g.tests_dir).join(format!("{}.c", g.tests[0])).is_file(), "{g:?}");
        }
    }
}
