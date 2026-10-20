//! The spec and test store: spec sheet skeletons from `specs/TEMPLATE.yaml`,
//! and the split of a spec agent's generated tests into visible tests
//! (committed under `specs/`) and held-out tests (kept outside the
//! repository, where implementer agents cannot see them).
//!
//! ```text
//! specs/<os>/<component>/<symbol>.yaml              spec sheet
//! specs/<os>/<component>/<symbol>.tests/<file>      visible tests (~70%)
//! $POLY_HELDOUT_DIR/<os>/<layer>/<component>/<symbol>/<file>          held out (~30%)
//! $POLY_HELDOUT_DIR/<os>/<layer>/<component>/<symbol>/MANIFEST.json   hashes of both
//! ```

use crate::sha256;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const HELDOUT_ENV: &str = "POLY_HELDOUT_DIR";
pub const MANIFEST: &str = "MANIFEST.json";

/// A coverage entry id, `<os>/<layer>/<component>/<symbol>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryId {
    pub os: String,
    pub layer: String,
    pub component: String,
    pub symbol: String,
}

impl EntryId {
    pub fn parse(id: &str) -> Result<EntryId> {
        let parts: Vec<&str> = id.splitn(4, '/').collect();
        let [os, layer, component, symbol] = parts.as_slice() else {
            return Err(format!("{id}: not an entry id (<os>/<layer>/<component>/<symbol>)"));
        };
        let ok = |s: &str| !s.is_empty() && !s.starts_with('.') && !s.contains(['/', '\\']);
        if !["linux", "windows", "macos"].contains(os) || !["syscall", "api"].contains(layer) {
            return Err(format!("{id}: unknown os or layer"));
        }
        if !ok(component) || !ok(symbol) {
            return Err(format!("{id}: component or symbol cannot be a file name"));
        }
        Ok(EntryId {
            os: os.to_string(),
            layer: layer.to_string(),
            component: component.to_string(),
            symbol: symbol.to_string(),
        })
    }

    pub fn id(&self) -> String {
        format!("{}/{}/{}/{}", self.os, self.layer, self.component, self.symbol)
    }

    /// The spec sheet, relative to the repository root.
    pub fn spec_path(&self) -> PathBuf {
        PathBuf::from("specs").join(&self.os).join(&self.component).join(format!("{}.yaml", self.symbol))
    }

    /// The visible tests directory, relative to the repository root.
    pub fn tests_dir(&self) -> PathBuf {
        PathBuf::from("specs").join(&self.os).join(&self.component).join(format!("{}.tests", self.symbol))
    }

    /// This entry's directory in the held-out store.
    pub fn heldout_dir(&self, heldout: &Path) -> PathBuf {
        heldout.join(&self.os).join(&self.layer).join(&self.component).join(&self.symbol)
    }
}

/// The held-out store: `$POLY_HELDOUT_DIR`, else `~/.poly-heldout`. It must
/// not be inside the repository.
pub fn heldout_root(repo: &Path) -> Result<PathBuf> {
    heldout_root_from(repo, std::env::var_os(HELDOUT_ENV), std::env::var_os("HOME"))
}

fn heldout_root_from(repo: &Path, var: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let dir = match var {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(home.ok_or("neither POLY_HELDOUT_DIR nor HOME is set")?).join(".poly-heldout"),
    };
    let dir = util::absolute(&dir);
    if util::is_inside(&dir, repo) || util::is_inside(repo, &dir) {
        return Err(format!(
            "the held-out store {} overlaps the repository {}; set {HELDOUT_ENV} to a directory outside it",
            dir.display(),
            repo.display()
        ));
    }
    Ok(dir)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHash {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeldOutManifest {
    pub format: String,
    pub entry: String,
    pub created: String,
    pub spec_agent: String,
    /// Repository HEAD when the tests were split.
    pub spec_revision: String,
    pub ratio: f64,
    /// Relative to the repository root.
    pub visible: Vec<FileHash>,
    /// Relative to the entry's held-out directory.
    pub held_out: Vec<FileHash>,
}

/// Top-level maps whose keys in the template are examples (one key per
/// case of the documented function), not fixed fields.
const FREE_FORM: &[&str] = &["returns", "errors"];

/// Fill the template: keys, structure and comments stay; the example
/// values become `TODO`, except what is known now. The example cases under
/// `returns` and `errors` become one `todo` key each.
pub fn spec_skeleton(template: &str, entry: &EntryId, oracle_os: &str, agent: &str, date: &str) -> String {
    let mut out = String::new();
    let mut block_indent: Option<usize> = None;
    for line in template.lines() {
        let indent = line.len() - line.trim_start().len();
        if let Some(i) = block_indent {
            if line.trim().is_empty() || indent > i {
                continue; // body of a `>` block or a free-form map: replaced
            }
            block_indent = None;
        }
        if indent == 0 && FREE_FORM.iter().any(|k| line.trim_end() == format!("{k}:")) {
            out.push_str(&format!(
                "{}\n  todo: TODO                                  # one key per case\n",
                line.trim_end()
            ));
            block_indent = Some(0);
            continue;
        }
        let trimmed = line.trim_start();
        let Some((key, rest)) = trimmed.split_once(':') else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        if trimmed.starts_with('#') || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let (value, comment) = match rest.find(" #") {
            Some(i) => (rest[..i].trim(), rest[i..].trim_end()),
            None => (rest.trim(), ""),
        };
        let pad = " ".repeat(indent);
        let new_value = match (key, value) {
            (_, "") => None,
            ("id", _) => Some(entry.id()),
            ("os", _) => Some(format!("\"{oracle_os}\"")),
            ("observed_by", _) => Some(agent.to_string()),
            ("date", _) => Some(date.to_string()),
            ("visible" | "held_out", _) => Some("0".into()),
            (_, "[]") => Some("[]".into()),
            (_, ">") => {
                block_indent = Some(indent);
                out.push_str(&format!("{pad}{key}: >{comment}\n{pad}  TODO\n"));
                continue;
            }
            _ => Some("TODO".into()),
        };
        match new_value {
            Some(v) => out.push_str(&format!("{pad}{key}: {v}")),
            None => out.push_str(&format!("{pad}{key}:")),
        }
        if !comment.is_empty() {
            let width = 46usize.saturating_sub(out.lines().last().map_or(0, str::len)).max(1);
            out.push_str(&" ".repeat(width));
            out.push_str(comment.trim_start());
        }
        out.push('\n');
    }
    out
}

/// The spec sheet with its `tests:` counts set.
pub fn set_test_counts(spec: &str, visible: usize, held_out: usize) -> String {
    let mut in_tests = false;
    let mut out = String::new();
    for line in spec.lines() {
        let trimmed = line.trim_start();
        if !line.starts_with(' ') && !trimmed.is_empty() {
            in_tests = trimmed.starts_with("tests:");
        }
        let replaced = if in_tests {
            [("visible:", visible), ("held_out:", held_out)].iter().find_map(|(key, n)| {
                let rest = trimmed.strip_prefix(key)?;
                let comment = rest.find('#').map_or(String::new(), |i| format!("  {}", &rest[i..]));
                Some(format!("{}{key} {n}{comment}", &line[..line.len() - trimmed.len()]))
            })
        } else {
            None
        };
        out.push_str(replaced.as_deref().unwrap_or(line));
        out.push('\n');
    }
    out
}

/// Which files are held out: a deterministic choice of about `ratio` of
/// them (at least one, never all), independent of their contents.
pub fn choose_held_out(entry: &str, names: &[String], ratio: f64) -> Vec<String> {
    let n = names.len();
    if n < 2 {
        return Vec::new();
    }
    let k = ((n as f64 * ratio).round() as usize).clamp(1, n - 1);
    let mut keyed: Vec<(String, &String)> =
        names.iter().map(|name| (sha256::hex(format!("{entry}\0{name}").as_bytes()), name)).collect();
    keyed.sort();
    let mut chosen: Vec<String> = keyed.into_iter().take(k).map(|(_, name)| name.clone()).collect();
    chosen.sort();
    chosen
}

#[derive(Debug)]
pub struct Split {
    pub visible: Vec<String>,
    pub held_out: Vec<String>,
    pub manifest_path: PathBuf,
}

/// What to split.
#[derive(Debug)]
pub struct SplitRequest<'a> {
    pub entry: &'a EntryId,
    /// Directory of generated test files, outside the repository.
    pub generated: &'a Path,
    /// Share of tests held out (about 0.3).
    pub ratio: f64,
    pub spec_agent: &'a str,
    pub revision: &'a str,
    /// Replace an earlier split of the same entry.
    pub replace: bool,
}

/// Split the tests in `generated` (a directory of files) for `entry`.
/// Visible ones are copied into the repository's tests directory, held-out
/// ones into the held-out store, and the manifest records both.
pub fn split_tests(repo: &Path, heldout: &Path, req: &SplitRequest<'_>) -> Result<Split> {
    let SplitRequest { entry, generated, ratio, spec_agent, revision, replace } = *req;
    if !(0.0..1.0).contains(&ratio) {
        return Err(format!("held-out ratio {ratio} must be in [0, 1)"));
    }
    if util::is_inside(generated, repo) {
        return Err(format!(
            "{} is inside the repository: generated tests must be written outside it, or the held-out ones \
             would be visible",
            generated.display()
        ));
    }
    let files = util::files_under(generated)?;
    if files.is_empty() {
        return Err(format!("no test files in {}", generated.display()));
    }
    let names: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let held: Vec<String> = choose_held_out(&entry.id(), &names, ratio);

    let mut hashes = BTreeMap::new();
    for name in &names {
        hashes.insert(name.clone(), sha256::hex(&util::read(&generated.join(name))?));
    }
    // The same content on both sides would make a visible file look like a
    // leaked held-out one (and would be one).
    for h in &held {
        if let Some(v) = names.iter().find(|v| !held.contains(v) && hashes[*v] == hashes[h]) {
            return Err(format!("{h} and {v} have the same content; remove the duplicate"));
        }
    }

    let tests_dir = repo.join(entry.tests_dir());
    let entry_dir = entry.heldout_dir(heldout);
    let manifest_path = entry_dir.join(MANIFEST);
    if !replace && (manifest_path.exists() || !util::files_under(&tests_dir)?.is_empty()) {
        return Err(format!("{} already has tests; pass --replace to split again", entry.id()));
    }
    if replace {
        for dir in [&tests_dir, &entry_dir] {
            if dir.exists() {
                std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
        }
    }

    let mut manifest = HeldOutManifest {
        format: "poly-heldout/1".into(),
        entry: entry.id(),
        created: util::now_utc(),
        spec_agent: spec_agent.into(),
        spec_revision: revision.into(),
        ratio,
        visible: Vec::new(),
        held_out: Vec::new(),
    };
    let (mut visible, mut held_out) = (Vec::new(), Vec::new());
    for name in &names {
        let data = util::read(&generated.join(name))?;
        if held.contains(name) {
            util::write(&entry_dir.join(name), &data)?;
            manifest.held_out.push(FileHash { path: name.clone(), sha256: hashes[name].clone() });
            held_out.push(name.clone());
        } else {
            let rel = entry.tests_dir().join(name);
            util::write(&repo.join(&rel), &data)?;
            manifest.visible.push(FileHash { path: rel.to_string_lossy().into_owned(), sha256: hashes[name].clone() });
            visible.push(name.clone());
        }
    }
    util::write_json(&manifest_path, &manifest)?;

    let spec = repo.join(entry.spec_path());
    if spec.exists() {
        util::write(&spec, set_test_counts(&util::read_string(&spec)?, visible.len(), held_out.len()))?;
    }
    Ok(Split { visible, held_out, manifest_path })
}

/// Content hash of every held-out test in the store -> where it is.
pub fn held_out_hashes(heldout: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for rel in util::files_under(heldout)? {
        if rel.file_name().is_some_and(|n| n == MANIFEST) {
            continue;
        }
        out.insert(sha256::hex(&util::read(&heldout.join(&rel))?), rel.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("poly-store-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn entry_ids() {
        let e = EntryId::parse("linux/syscall/x86_64/write").unwrap();
        assert_eq!(e.spec_path(), Path::new("specs/linux/x86_64/write.yaml"));
        assert_eq!(e.tests_dir(), Path::new("specs/linux/x86_64/write.tests"));
        assert_eq!(e.heldout_dir(Path::new("/h")), Path::new("/h/linux/syscall/x86_64/write"));
        assert!(EntryId::parse("linux/syscall/write").is_err());
        assert!(EntryId::parse("plan9/syscall/x/y").is_err());
        assert!(EntryId::parse("linux/syscall/../x").is_err());
    }

    #[test]
    fn skeleton_keeps_every_template_key() {
        let template = include_str!("../../../specs/TEMPLATE.yaml");
        let e = EntryId::parse("linux/syscall/x86_64/write").unwrap();
        let s = spec_skeleton(template, &e, "Linux 6.8.0", "spec-agent-1", "2026-10-20");
        // Every top-level key, and every fixed nested one, in order.
        let keys = |t: &str| -> Vec<String> {
            let mut parent = String::new();
            let mut out = Vec::new();
            for l in t.lines().filter(|l| !l.trim_start().starts_with('#')) {
                let Some((k, _)) = l.split_once(':') else { continue };
                if !k.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                if !l.starts_with(' ') {
                    parent = k.to_string();
                } else if FREE_FORM.contains(&parent.as_str()) {
                    continue;
                }
                out.push(k.to_string());
            }
            out
        };
        assert_eq!(keys(&s), keys(template));
        assert!(s.contains("returns:\n  todo: TODO") && s.contains("errors:\n  todo: TODO"), "{s}");
        assert!(s.contains("id: linux/syscall/x86_64/write"), "{s}");
        assert!(s.contains("  os: \"Linux 6.8.0\""));
        assert!(s.contains("  observed_by: spec-agent-1"));
        assert!(s.contains("summary: >\n  TODO\n"));
        assert!(!s.contains("FindFirstFileW") && !s.contains("NTFS") && !s.contains("path_not_found"), "{s}");
        assert!(s.contains("# count in <symbol>.tests/"), "comments stay");
    }

    #[test]
    fn test_counts_are_updated_in_place() {
        let spec = "id: x\ntests:\n  visible: 0          # count\n  held_out: 0\nother:\n  visible: 9\n";
        assert_eq!(
            set_test_counts(spec, 7, 3),
            "id: x\ntests:\n  visible: 7  # count\n  held_out: 3\nother:\n  visible: 9\n"
        );
    }

    #[test]
    fn held_out_choice_is_stable_and_bounded() {
        let names: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        let a = choose_held_out("e", &names, 0.3);
        assert_eq!(a.len(), 3);
        assert_eq!(a, choose_held_out("e", &names, 0.3));
        assert_ne!(a, choose_held_out("other", &names, 0.3), "depends on the entry");
        assert_eq!(choose_held_out("e", &names[..2], 0.01).len(), 1, "at least one");
        assert_eq!(choose_held_out("e", &names[..2], 0.99).len(), 1, "never all");
        assert!(choose_held_out("e", &names[..1], 0.3).is_empty());
    }

    #[test]
    fn split_puts_held_out_tests_outside_the_repo() {
        let base = temp("split");
        let (repo, heldout, generated) = (base.join("repo"), base.join("heldout"), base.join("gen"));
        for i in 0..10 {
            util::write(&generated.join(format!("case{i}.c")), format!("test {i}")).unwrap();
        }
        let e = EntryId::parse("linux/syscall/x86_64/write").unwrap();
        util::write(&repo.join(e.spec_path()), "tests:\n  visible: 0\n  held_out: 0\n").unwrap();
        let mut req = SplitRequest {
            entry: &e,
            generated: &generated,
            ratio: 0.3,
            spec_agent: "spec-1",
            revision: "abc",
            replace: false,
        };
        let s = split_tests(&repo, &heldout, &req).unwrap();
        assert_eq!((s.visible.len(), s.held_out.len()), (7, 3));
        for h in &s.held_out {
            assert!(!repo.join(e.tests_dir()).join(h).exists());
            assert!(e.heldout_dir(&heldout).join(h).exists());
        }
        let m: HeldOutManifest = util::read_json(&s.manifest_path).unwrap();
        assert_eq!(m.visible[0].path, format!("specs/linux/x86_64/write.tests/{}", s.visible[0]));
        assert_eq!(held_out_hashes(&heldout).unwrap().len(), 3);
        assert_eq!(util::read_string(&repo.join(e.spec_path())).unwrap(), "tests:\n  visible: 7\n  held_out: 3\n");
        // A second split needs --replace.
        assert!(split_tests(&repo, &heldout, &req).is_err());
        req.replace = true;
        assert!(split_tests(&repo, &heldout, &req).is_ok());
        // Generated tests inside the repository are refused.
        let inside = repo.join("specs");
        req.generated = &inside;
        assert!(split_tests(&repo, &heldout, &req).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn duplicate_content_across_the_split_is_refused() {
        let base = temp("dup");
        for i in 0..4 {
            util::write(&base.join("gen").join(format!("t{i}")), "same").unwrap();
        }
        let e = EntryId::parse("linux/syscall/x86_64/brk").unwrap();
        let generated = base.join("gen");
        let req = SplitRequest {
            entry: &e,
            generated: &generated,
            ratio: 0.3,
            spec_agent: "s",
            revision: "r",
            replace: false,
        };
        let r = split_tests(&base.join("repo"), &base.join("h"), &req);
        assert!(r.unwrap_err().contains("same content"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn heldout_store_must_not_overlap_the_repo() {
        let repo = crate::repo_root();
        let inside = Some(repo.join("target/heldout").into_os_string());
        assert!(heldout_root_from(&repo, inside, None).is_err());
        let outside = Some(std::env::temp_dir().join("poly-heldout-x").into_os_string());
        assert!(heldout_root_from(&repo, outside, None).is_ok());
        let home = Some(std::env::temp_dir().into_os_string());
        assert!(heldout_root_from(&repo, None, home).unwrap().ends_with(".poly-heldout"));
        // The repository inside the store is just as bad.
        assert!(heldout_root_from(&repo, Some("/".into()), None).is_err());
    }
}
