//! The hand-off log `specs/HANDOFF_LOG.md`: one table row per hand-off from
//! a spec agent to an implementer agent, so the role separation in
//! CLEAN_ROOM.md can be proven later.
//!
//! A row records the date (UTC), the entry, the git revision the spec was
//! committed at, both agent ids, and every file handed over with its
//! SHA-256. `verify` re-reads each file from git at that revision and
//! checks the hashes, that no handed-over file (or anything committed
//! under `specs/`, or anything an implementer received) has the content of
//! a held-out test, and that no agent played both roles for an entry.

use crate::sha256;
use crate::store::{self, EntryId, FileHash};
use crate::util::{self, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const LOG: &str = "specs/HANDOFF_LOG.md";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub date: String,
    pub entry: String,
    pub revision: String,
    pub spec_agent: String,
    pub implementer: String,
    pub files: Vec<FileHash>,
}

impl Record {
    /// The table row.
    pub fn row(&self) -> String {
        let files: Vec<String> = self.files.iter().map(|f| format!("`{}` sha256:{}", f.path, f.sha256)).collect();
        format!(
            "| {} | {} | {} | {} | {} | {} |",
            self.date,
            self.entry,
            self.revision,
            self.spec_agent,
            self.implementer,
            files.join("<br>")
        )
    }
}

fn parse_files(cell: &str) -> Result<Vec<FileHash>> {
    cell.split("<br>")
        .filter(|s| !s.trim().is_empty())
        .map(|item| {
            let item = item.trim();
            let (path, hash) = item
                .strip_prefix('`')
                .and_then(|r| r.split_once("` sha256:"))
                .ok_or_else(|| format!("bad file item {item:?}"))?;
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("bad hash in {item:?}"));
            }
            Ok(FileHash { path: path.to_string(), sha256: hash.to_string() })
        })
        .collect()
}

/// Every record in the log.
pub fn parse_log(text: &str) -> Result<Vec<Record>> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if !line.starts_with('|') || line.starts_with("|---") || line.starts_with("| Date") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split(" | ").map(str::trim).collect();
        let [date, entry, revision, spec_agent, implementer, files] = cells.as_slice() else {
            return Err(format!("{LOG}:{}: expected 6 columns", n + 1));
        };
        out.push(Record {
            date: date.to_string(),
            entry: entry.to_string(),
            revision: revision.to_string(),
            spec_agent: spec_agent.to_string(),
            implementer: implementer.to_string(),
            files: parse_files(files).map_err(|e| format!("{LOG}:{}: {e}", n + 1))?,
        });
    }
    Ok(out)
}

fn load(repo: &Path) -> Result<(String, Vec<Record>)> {
    let text = util::read_string(&repo.join(LOG))?;
    let records = parse_log(&text)?;
    Ok((text, records))
}

/// Role separation (CLEAN_ROOM.md, rule 4): the two roles are different
/// agents, and nobody switches roles for the same entry.
fn role_problems(record: &Record, all: &[Record]) -> Vec<String> {
    let mut out = Vec::new();
    if record.spec_agent == record.implementer {
        out.push(format!("{}: {} is both spec agent and implementer", record.entry, record.spec_agent));
    }
    for other in all.iter().filter(|o| o.entry == record.entry) {
        if other.implementer == record.spec_agent {
            out.push(format!("{}: spec agent {} was an implementer of it", record.entry, record.spec_agent));
        }
        if other.spec_agent == record.implementer {
            out.push(format!("{}: implementer {} was a spec agent of it", record.entry, record.implementer));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The files of a hand-off for `entry`: the spec sheet and the visible tests.
pub fn handoff_files(repo: &Path, entry: &EntryId) -> Result<Vec<PathBuf>> {
    let spec = entry.spec_path();
    if !repo.join(&spec).is_file() {
        return Err(format!("{} does not exist; create it with `pipeline spec new`", spec.display()));
    }
    let mut files = vec![spec];
    for rel in util::files_under(&repo.join(entry.tests_dir()))? {
        files.push(entry.tests_dir().join(rel));
    }
    for f in &files {
        let s = f.to_string_lossy();
        if s.contains(['|', '`']) || s.contains("<br>") {
            return Err(format!("{s}: file names in a hand-off cannot contain '|', '`' or \"<br>\""));
        }
    }
    Ok(files)
}

#[derive(Debug)]
pub struct RecordRequest<'a> {
    pub entry: &'a EntryId,
    pub spec_agent: &'a str,
    pub implementer: &'a str,
    /// Allow uncommitted changes to the files (the revision then does not
    /// contain what was handed over, and `verify` will say so).
    pub allow_dirty: bool,
    /// The held-out store, if there is one on this machine.
    pub heldout: Option<&'a Path>,
}

/// Append a hand-off record to the log and return it.
pub fn record(repo: &Path, req: &RecordRequest<'_>) -> Result<Record> {
    let files = handoff_files(repo, req.entry)?;
    let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    if !req.allow_dirty {
        let mut args = vec!["status", "--porcelain", "--"];
        args.extend(paths.iter().map(String::as_str));
        let dirty = util::git_text(repo, &args)?;
        if !dirty.is_empty() {
            return Err(format!("commit the spec and visible tests first:\n{dirty}"));
        }
    }
    let revision = util::git_text(repo, &["rev-parse", "HEAD"])?;
    let mut hashes = Vec::new();
    for p in &paths {
        hashes.push(FileHash { path: p.clone(), sha256: sha256::hex(&util::read(&repo.join(p))?) });
    }
    let record = Record {
        date: util::now_utc(),
        entry: req.entry.id(),
        revision,
        spec_agent: req.spec_agent.into(),
        implementer: req.implementer.into(),
        files: hashes,
    };
    for agent in [req.spec_agent, req.implementer] {
        if agent.is_empty() || agent.contains(['|', '`']) || agent.contains(char::is_whitespace) {
            return Err(format!("agent id {agent:?} must be non-empty without spaces, '|' or '`'"));
        }
    }
    let (text, existing) = load(repo)?;
    let problems = role_problems(&record, &existing);
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    if let Some(heldout) = req.heldout {
        let held = store::held_out_hashes(heldout)?;
        for f in &record.files {
            if let Some(h) = held.get(&f.sha256) {
                return Err(format!("{} has the content of held-out test {h}: refusing to hand it over", f.path));
            }
        }
    }
    let mut text = text;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&record.row());
    text.push('\n');
    util::write(&repo.join(LOG), text)?;
    Ok(record)
}

/// The latest hand-off of `entry` in the log.
pub fn latest(repo: &Path, entry: &str) -> Result<Record> {
    load(repo)?.1.into_iter().rev().find(|r| r.entry == entry).ok_or_else(|| format!("no hand-off of {entry} in {LOG}"))
}

fn file_at(repo: &Path, revision: &str, path: &str) -> Result<Vec<u8>> {
    util::git(repo, &["show", &format!("{revision}:{path}")])
}

/// Write the files of the latest hand-off of `entry`, as committed at its
/// revision, into `out`: everything an implementer agent receives.
pub fn bundle(repo: &Path, entry: &str, out: &Path, heldout: Option<&Path>) -> Result<Record> {
    if let Some(h) = heldout
        && (util::is_inside(out, h) || util::is_inside(h, out))
    {
        return Err(format!("{} overlaps the held-out store", out.display()));
    }
    if out.exists() && !util::files_under(out)?.is_empty() {
        return Err(format!("{} is not empty", out.display()));
    }
    let record = latest(repo, entry)?;
    for f in &record.files {
        let data = file_at(repo, &record.revision, &f.path)?;
        if sha256::hex(&data) != f.sha256 {
            return Err(format!("{} at {} does not match the logged hash", f.path, record.revision));
        }
        util::write(&out.join(&f.path), data)?;
    }
    Ok(record)
}

#[derive(Debug, Default)]
pub struct VerifyRequest<'a> {
    /// Only this entry (all records otherwise).
    pub entry: Option<&'a str>,
    /// The held-out store, if available here.
    pub heldout: Option<&'a Path>,
    /// What an implementer actually received (its input directory); needs `entry`.
    pub received: Option<&'a Path>,
}

/// Every problem found; empty means the hand-offs check out.
pub fn verify(repo: &Path, req: &VerifyRequest<'_>) -> Result<Vec<String>> {
    let (_, all) = load(repo)?;
    let records: Vec<&Record> = all.iter().filter(|r| req.entry.is_none_or(|e| r.entry == e)).collect();
    let mut problems = Vec::new();
    if let Some(e) = req.entry
        && records.is_empty()
    {
        problems.push(format!("no hand-off of {e} in {LOG}"));
    }
    let held: BTreeMap<String, String> = match req.heldout {
        Some(h) => store::held_out_hashes(h)?,
        None => BTreeMap::new(),
    };
    for r in &records {
        problems.extend(role_problems(r, &all));
        for f in &r.files {
            match file_at(repo, &r.revision, &f.path) {
                Ok(data) if sha256::hex(&data) == f.sha256 => {}
                Ok(_) => {
                    problems.push(format!("{}: {} at {} differs from the logged hash", r.entry, f.path, r.revision))
                }
                Err(_) => problems.push(format!("{}: {} is not in git at {}", r.entry, f.path, r.revision)),
            }
            if let Some(h) = held.get(&f.sha256) {
                problems.push(format!("{}: handed-over file {} is held-out test {h}", r.entry, f.path));
            }
        }
    }
    // Nothing committed under specs/ may be a held-out test.
    if !held.is_empty() {
        for rel in util::files_under(&repo.join("specs"))? {
            let data = util::read(&repo.join("specs").join(&rel))?;
            if let Some(h) = held.get(&sha256::hex(&data)) {
                problems.push(format!("specs/{} is held-out test {h}", rel.display()));
            }
        }
    }
    if let Some(dir) = req.received {
        let entry = req.entry.ok_or("--received needs the entry id")?;
        let r = records.last().ok_or_else(|| format!("no hand-off of {entry}"))?;
        let logged: BTreeMap<&str, &str> = r.files.iter().map(|f| (f.sha256.as_str(), f.path.as_str())).collect();
        let mut seen = Vec::new();
        for rel in util::files_under(dir)? {
            let hash = sha256::hex(&util::read(&dir.join(&rel))?);
            if let Some(h) = held.get(&hash) {
                problems.push(format!("received {} is held-out test {h}: LEAK", rel.display()));
            } else if !logged.contains_key(hash.as_str()) {
                problems.push(format!("received {} was not in the logged hand-off", rel.display()));
            }
            seen.push(hash);
        }
        for f in &r.files {
            if !seen.contains(&f.sha256) {
                problems.push(format!("logged {} was not received", f.path));
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{SplitRequest, split_tests};
    use std::process::Command;

    const HEADER: &str = "# Hand-off log\n\n| Date (UTC) | Entry id | Spec revision (git) | Spec agent | Implementer agent | Files handed over |\n|---|---|---|---|---|---|\n";

    /// A scratch git repository with a spec, ten generated tests split
    /// 7/3, everything committed. Returns (base, repo, heldout).
    fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("poly-handoff-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (repo, heldout, generated) = (base.join("repo"), base.join("heldout"), base.join("gen"));
        util::write(&repo.join(LOG), HEADER).unwrap();
        let e = EntryId::parse("linux/syscall/x86_64/write").unwrap();
        util::write(&repo.join(e.spec_path()), "id: linux/syscall/x86_64/write\ntests:\n  visible: 0\n  held_out: 0\n")
            .unwrap();
        for i in 0..10 {
            util::write(&generated.join(format!("t{i}.c")), format!("case {i}")).unwrap();
        }
        let req = SplitRequest {
            entry: &e,
            generated: &generated,
            ratio: 0.3,
            spec_agent: "spec-1",
            revision: "x",
            replace: false,
        };
        split_tests(&repo, &heldout, &req).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q"]);
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "spec"]);
        (base, repo, heldout)
    }

    fn entry() -> EntryId {
        EntryId::parse("linux/syscall/x86_64/write").unwrap()
    }

    fn rec<'a>(e: &'a EntryId, heldout: &'a Path, spec: &'a str, implementer: &'a str) -> RecordRequest<'a> {
        RecordRequest { entry: e, spec_agent: spec, implementer, allow_dirty: false, heldout: Some(heldout) }
    }

    #[test]
    fn rows_roundtrip() {
        let r = Record {
            date: "2026-10-20 10:00:00".into(),
            entry: "linux/syscall/x86_64/write".into(),
            revision: "abc".into(),
            spec_agent: "s".into(),
            implementer: "i".into(),
            files: vec![
                FileHash { path: "specs/a.yaml".into(), sha256: "0".repeat(64) },
                FileHash { path: "specs/a.tests/t".into(), sha256: "f".repeat(64) },
            ],
        };
        let text = format!("{HEADER}{}\n", r.row());
        assert_eq!(parse_log(&text).unwrap(), [r]);
        assert!(parse_log(&format!("{HEADER}| a | b |\n")).is_err());
        assert!(parse_log(&format!("{HEADER}| a | b | c | d | e | `x` sha256:12 |\n")).is_err());
    }

    #[test]
    fn record_bundle_and_verify() {
        let (base, repo, heldout) = setup("ok");
        let e = entry();
        let r = record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap();
        assert_eq!(r.files.len(), 8, "spec + 7 visible tests");
        assert_eq!(r.files[0].path, "specs/linux/x86_64/write.yaml");
        let out = base.join("bundle");
        bundle(&repo, &e.id(), &out, Some(&heldout)).unwrap();
        let req =
            VerifyRequest { entry: Some("linux/syscall/x86_64/write"), heldout: Some(&heldout), received: Some(&out) };
        assert_eq!(verify(&repo, &req).unwrap(), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_catches_a_leaked_held_out_file() {
        let (base, repo, heldout) = setup("leak");
        let e = entry();
        record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap();
        let out = base.join("bundle");
        bundle(&repo, &e.id(), &out, Some(&heldout)).unwrap();
        // Smuggle a held-out test into what the implementer received, renamed.
        let held = store::held_out_hashes(&heldout).unwrap();
        let (_, leaked) = held.iter().next().unwrap();
        std::fs::copy(heldout.join(leaked), out.join("innocent.c")).unwrap();
        let req =
            VerifyRequest { entry: Some("linux/syscall/x86_64/write"), heldout: Some(&heldout), received: Some(&out) };
        let problems = verify(&repo, &req).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("received innocent.c is held-out test") && problems[0].ends_with("LEAK"));

        // And one committed into the visible tests.
        std::fs::copy(heldout.join(leaked), repo.join(e.tests_dir()).join("copied.c")).unwrap();
        let problems = verify(&repo, &VerifyRequest { heldout: Some(&heldout), ..Default::default() }).unwrap();
        assert!(problems.iter().any(|p| p.contains("copied.c is held-out test")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn record_refuses_held_out_content_and_dirty_files() {
        let (base, repo, heldout) = setup("refuse");
        let e = entry();
        let held = store::held_out_hashes(&heldout).unwrap();
        let (_, leaked) = held.iter().next().unwrap();
        std::fs::copy(heldout.join(leaked), repo.join(e.tests_dir()).join("copied.c")).unwrap();
        let err = record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap_err();
        assert!(err.contains("commit the spec"), "{err}");
        let dirty = RecordRequest { allow_dirty: true, ..rec(&e, &heldout, "spec-1", "impl-1") };
        let err = record(&repo, &dirty).unwrap_err();
        assert!(err.contains("held-out test"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn roles_stay_separate() {
        let (base, repo, heldout) = setup("roles");
        let e = entry();
        assert!(record(&repo, &rec(&e, &heldout, "a", "a")).unwrap_err().contains("both"));
        record(&repo, &rec(&e, &heldout, "a", "b")).unwrap();
        // b implemented it, so b cannot write its spec later; nor a implement it.
        assert!(record(&repo, &rec(&e, &heldout, "b", "c")).unwrap_err().contains("was an implementer"));
        assert!(record(&repo, &rec(&e, &heldout, "c", "a")).unwrap_err().contains("was a spec agent"));
        record(&repo, &rec(&e, &heldout, "a", "c")).unwrap();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_catches_a_changed_file() {
        let (base, repo, heldout) = setup("changed");
        let e = entry();
        record(&repo, &rec(&e, &heldout, "s", "i")).unwrap();
        // Tamper with the log: the hash of the spec no longer matches git.
        let log = util::read_string(&repo.join(LOG)).unwrap();
        let spec_hash = sha256::hex(&util::read(&repo.join(e.spec_path())).unwrap());
        util::write(&repo.join(LOG), log.replace(&spec_hash, &"0".repeat(64))).unwrap();
        let problems = verify(&repo, &VerifyRequest::default()).unwrap();
        assert!(problems.iter().any(|p| p.contains("differs from the logged hash")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
