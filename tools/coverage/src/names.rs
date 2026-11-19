//! Windows test names are unique (`coverage check`).
//!
//! `pipeline win build` compiles every test of the corpus (`tests/windows`) and of the entries it is
//! given into one Cargo package, one binary per file, so two files with the same stem cannot be built
//! together, and the recorded results (`windows.json`) are looked up by name. A duplicate stays hidden
//! until the day both entries are built in one run (it ended a CI run once: `uc_basic` of
//! `UrlCanonicalizeW` and of `UnregisterClassW`). This check finds them when the second one is added.
//!
//! Sources of names: the files of `tests/windows/*.rs|cpp` (the corpus), of every
//! `specs/windows/**/*.tests/` directory (visible tests), and, when the held-out store is there
//! (`$POLY_HELDOUT_DIR`, else `~/.poly-heldout`; CI has none), the names the manifests of the entries
//! list as held out. Only the manifests are read, never a held-out test.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file extensions of a test source (as `pipeline win build` takes them).
const TEST_EXTENSIONS: &[&str] = &["rs", "cpp"];

/// Where a test name comes from, for the message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin(pub String);

/// Every name used by more than one origin, with its origins, as one message per name.
/// An origin that lists a name twice (a `.rs` and a `.cpp` of one entry) counts once per origin.
pub fn duplicates(names: &[(String, Origin)]) -> Vec<String> {
    let mut by_name: BTreeMap<&str, Vec<&Origin>> = BTreeMap::new();
    for (name, origin) in names {
        by_name.entry(name).or_default().push(origin);
    }
    let mut out = Vec::new();
    for (name, mut origins) in by_name {
        origins.sort();
        origins.dedup();
        if origins.len() > 1 {
            let list: Vec<&str> = origins.iter().map(|o| o.0.as_str()).collect();
            out.push(format!("test name `{name}` is used by {} entries: {}", list.len(), list.join(", ")));
        }
    }
    out
}

fn stem_of_test(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    if !TEST_EXTENSIONS.contains(&ext) {
        return None;
    }
    Some(path.file_stem()?.to_string_lossy().into_owned())
}

/// The visible test names below `specs/windows` and the corpus names of `tests/windows`.
pub fn visible_names(root: &Path) -> Result<Vec<(String, Origin)>, String> {
    let mut out = Vec::new();
    let corpus = root.join("tests/windows");
    if corpus.is_dir() {
        for e in std::fs::read_dir(&corpus).map_err(|e| format!("{}: {e}", corpus.display()))? {
            let path = e.map_err(|e| e.to_string())?.path();
            if path.is_file() {
                if let Some(n) = stem_of_test(&path) {
                    out.push((n, Origin("the corpus (tests/windows)".into())));
                }
            }
        }
    }
    let specs = root.join("specs/windows");
    if specs.is_dir() {
        walk_tests(root, &specs, &mut out)?;
    }
    Ok(out)
}

fn walk_tests(root: &Path, dir: &Path, out: &mut Vec<(String, Origin)>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = e.map_err(|e| e.to_string())?.path();
        if !path.is_dir() {
            continue;
        }
        if path.extension().is_some_and(|x| x == "tests") {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let entry = rel.trim_end_matches(".tests").to_string();
            for f in std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))? {
                let f = f.map_err(|e| e.to_string())?.path();
                if f.is_file() {
                    if let Some(n) = stem_of_test(&f) {
                        out.push((n, Origin(entry.clone())));
                    }
                }
            }
        } else {
            walk_tests(root, &path, out)?;
        }
    }
    Ok(())
}

/// The held-out store, if there is one: `$POLY_HELDOUT_DIR`, else `~/.poly-heldout` (`%USERPROFILE%` on Windows).
pub fn heldout_store(repo: &Path) -> Option<PathBuf> {
    let dir = match std::env::var_os("POLY_HELDOUT_DIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            PathBuf::from(home).join(".poly-heldout")
        }
    };
    // A store inside the repository is refused by the pipeline; do not read one here.
    if dir.starts_with(repo) || !dir.is_dir() {
        return None;
    }
    Some(dir)
}

/// The held-out test names of the Windows entries in `store`, from the manifests only.
pub fn heldout_names(store: &Path) -> Vec<(String, Origin)> {
    let mut out = Vec::new();
    let windows = store.join("windows");
    let mut manifests = Vec::new();
    collect_manifests(&windows, &mut manifests, 0);
    for m in manifests {
        let Ok(text) = std::fs::read_to_string(&m) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let entry = v.get("entry").and_then(|e| e.as_str()).unwrap_or("?").to_string();
        for h in v.get("held_out").and_then(|h| h.as_array()).into_iter().flatten() {
            if let Some(p) = h.get("path").and_then(|p| p.as_str()) {
                if let Some(n) = stem_of_test(Path::new(p)) {
                    out.push((n, Origin(format!("{entry} (held out)"))));
                }
            }
        }
    }
    out
}

fn collect_manifests(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 5 {
            collect_manifests(&p, out, depth + 1);
        } else if p.file_name().is_some_and(|n| n == "MANIFEST.json") {
            out.push(p);
        }
    }
}

/// All duplicate-name messages for the repository at `root` (empty: fine). Held-out names are included
/// when the store is present.
pub fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut names = visible_names(root)?;
    if let Some(store) = heldout_store(root) {
        names.extend(heldout_names(&store));
    }
    Ok(duplicates(&names))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(name: &str, origin: &str) -> (String, Origin) {
        (name.to_string(), Origin(origin.to_string()))
    }

    #[test]
    fn a_name_in_two_entries_is_reported_with_both() {
        let d = duplicates(&[n("uc_basic", "shlwapi/UrlCanonicalizeW"), n("other", "a/B"), n("uc_basic", "user32/UnregisterClassW")]);
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("`uc_basic`") && d[0].contains("shlwapi/UrlCanonicalizeW") && d[0].contains("user32/UnregisterClassW"), "{d:?}");
    }

    #[test]
    fn one_entry_listing_a_name_twice_is_no_duplicate() {
        // a `.rs` and a `.cpp` of one entry are one origin; the build refuses them elsewhere
        assert!(duplicates(&[n("t", "a/B"), n("t", "a/B")]).is_empty());
    }

    #[test]
    fn a_held_out_name_collides_with_a_visible_one() {
        let d = duplicates(&[n("x", "a/B"), n("x", "c/D (held out)")]);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn unique_names_are_fine() {
        assert!(duplicates(&[n("a", "e1"), n("b", "e2"), n("c", "e2")]).is_empty());
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("poly-names-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn the_scan_finds_visible_tests_below_specs_and_the_corpus() {
        let root = scratch("scan");
        put(&root, "specs/windows/shlwapi.dll/UrlCanonicalizeW.tests/uc_basic.rs");
        put(&root, "specs/windows/shlwapi.dll/UrlCanonicalizeW.tests/windows.json");
        put(&root, "specs/windows/user32.dll/UnregisterClassW.tests/uc_basic.rs");
        put(&root, "specs/windows/user32.dll/UnregisterClassW.yaml");
        put(&root, "tests/windows/nt_file_rw.rs");
        put(&root, "tests/windows/support/src/lib.rs");
        let names = visible_names(&root).unwrap();
        let mut got: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
        got.sort();
        assert_eq!(got, ["nt_file_rw", "uc_basic", "uc_basic"]);
        let d = duplicates(&names);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("specs/windows/shlwapi.dll/UrlCanonicalizeW") && d[0].contains("specs/windows/user32.dll/UnregisterClassW"), "{d:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corpus_name_collides_with_a_spec_test() {
        let root = scratch("corpus");
        put(&root, "tests/windows/same.rs");
        put(&root, "specs/windows/k.dll/E.tests/same.rs");
        let d = duplicates(&visible_names(&root).unwrap());
        assert_eq!(d.len(), 1, "{d:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_held_out_names_come_from_the_manifests_only() {
        let store = scratch("store");
        let dir = store.join("windows/api/k.dll/E");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("MANIFEST.json"),
            r#"{"format":"poly-heldout/1","entry":"windows/api/k.dll/E","visible":[{"path":"specs/windows/k.dll/E.tests/v.rs","sha256":"0"}],"held_out":[{"path":"h_one.rs","sha256":"0"},{"path":"windows.json","sha256":"0"}]}"#,
        )
        .unwrap();
        // the test file itself is not even present: the check must not need it
        let names = heldout_names(&store);
        assert_eq!(names.len(), 1, "{names:?}");
        assert_eq!(names[0].0, "h_one");
        assert!(names[0].1.0.contains("windows/api/k.dll/E"));
        let _ = std::fs::remove_dir_all(&store);
    }
}
