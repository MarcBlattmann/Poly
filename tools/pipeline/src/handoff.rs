//! Hand-off records: who handed which spec sheet and visible tests to whom,
//! so the role separation in CLEAN_ROOM.md can be proven later.
//!
//! A record holds the date (UTC), the entry, the git revision the spec was
//! committed at, both agent ids, and every file handed over with its
//! SHA-256. There are two places for them (ADR 0213):
//!
//! - `specs/handoffs/`: one file per record, named
//!   `<UTC timestamp>-<spec agent>-<implementer>-<entry hash>.md`. `record`
//!   writes these. Two pull requests then never edit the same file, so
//!   landing one after the other does not conflict.
//! - `specs/HANDOFF_LOG.md`: the legacy table, one row per record. Nothing
//!   writes to it any more, but it is still read (and open pull requests
//!   still append to it), and every record in it is checked the same way.
//!
//! `verify` reads the files of every record from git at its revision (in
//! one `git cat-file --batch`, not a process per file) and checks the
//! hashes, that no handed-over file (or anything committed under `specs/`,
//! or anything an implementer received) has the content of a held-out test,
//! and that no agent played both roles for an entry.

use crate::sha256;
use crate::store::{self, EntryId, FileHash};
use crate::util::{self, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The legacy table (ADR 0027); read, no longer written.
pub const LOG: &str = "specs/HANDOFF_LOG.md";

/// The directory with one file per record (ADR 0213).
pub const DIR: &str = "specs/handoffs";

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
    /// The table row (the legacy log's; kept to test that it is still read).
    #[cfg(test)]
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

impl Record {
    /// The file name under [`DIR`]: `<UTC timestamp>-<spec agent>-<implementer>-<entry hash>.md`.
    /// The timestamp sorts in time order; the hash (of the entry id) keeps two
    /// entries handed over in the same second apart.
    pub fn file_name(&self) -> String {
        let stamp = self.date.replace(['-', ':'], "").replace(' ', "T");
        let entry_hash = &sha256::hex(self.entry.as_bytes())[..12];
        format!("{stamp}Z-{}-{}-{entry_hash}.md", self.spec_agent, self.implementer)
    }

    /// The contents of the record's file: the same fields as a log row.
    pub fn file_text(&self) -> String {
        let mut out = format!(
            "# Hand-off of {}\n\nDate (UTC): {}\nEntry id: {}\nSpec revision (git): {}\nSpec agent: {}\nImplementer agent: {}\n\nFiles handed over:\n\n",
            self.entry, self.date, self.entry, self.revision, self.spec_agent, self.implementer
        );
        for f in &self.files {
            out.push_str(&format!("- `{}` sha256:{}\n", f.path, f.sha256));
        }
        out
    }
}

/// A record from the text of its file ([`Record::file_text`]).
pub fn parse_file(text: &str) -> Result<Record> {
    let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
    let mut files = Vec::new();
    let mut in_files = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with("# ") {
            continue;
        }
        if line == "Files handed over:" {
            in_files = true;
        } else if in_files {
            let item = line.strip_prefix("- ").ok_or_else(|| format!("line {}: expected a file item", n + 1))?;
            files.extend(parse_files(item).map_err(|e| format!("line {}: {e}", n + 1))?);
        } else {
            let (key, value) =
                line.split_once(": ").ok_or_else(|| format!("line {}: expected `Name: value`", n + 1))?;
            if !FIELDS.contains(&key) || fields.insert(key, value.trim()).is_some() {
                return Err(format!("line {}: unknown or repeated field {key:?}", n + 1));
            }
        }
    }
    let get = |k: &str| fields.get(k).map(|v| v.to_string()).ok_or_else(|| format!("missing {k:?}"));
    if !in_files || files.is_empty() {
        return Err("no files handed over".into());
    }
    Ok(Record {
        date: get("Date (UTC)")?,
        entry: get("Entry id")?,
        revision: get("Spec revision (git)")?,
        spec_agent: get("Spec agent")?,
        implementer: get("Implementer agent")?,
        files,
    })
}

const FIELDS: [&str; 5] = ["Date (UTC)", "Entry id", "Spec revision (git)", "Spec agent", "Implementer agent"];

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

/// What [`load`] finds: the records, and the problems of the files' names, each with its entry.
type Loaded = (Vec<Record>, Vec<(String, String)>);

/// Every record: the legacy log's rows in their order, then the files under
/// [`DIR`] by date (and file name). Problems with the files' names come too,
/// each with the entry of its record.
fn load(repo: &Path) -> Result<Loaded> {
    let log = repo.join(LOG);
    let mut records = if log.is_file() { parse_log(&util::read_string(&log)?)? } else { Vec::new() };
    let mut problems = Vec::new();
    let mut from_files = Vec::new();
    for rel in util::files_under(&repo.join(DIR))? {
        let name = rel.to_string_lossy().into_owned();
        // A README for the directory is not a record; nothing in a sub-directory is read.
        if name == "README.md" || !name.ends_with(".md") || name.contains('/') {
            continue;
        }
        let text = util::read_string(&repo.join(DIR).join(&rel))?;
        let record = parse_file(&text).map_err(|e| format!("{DIR}/{name}: {e}"))?;
        if record.file_name() != name {
            let problem =
                format!("{DIR}/{name}: the file name does not match its record (it should be {})", record.file_name());
            problems.push((record.entry.clone(), problem));
        }
        from_files.push((name, record));
    }
    from_files.sort_by(|a, b| (&a.1.date, &a.0).cmp(&(&b.1.date, &b.0)));
    records.extend(from_files.into_iter().map(|(_, r)| r));
    Ok((records, problems))
}

/// Role separation (CLEAN_ROOM.md, rule 4): the two roles are different
/// agents, and nobody switches roles for the same entry.
fn role_problems(record: &Record, all: &[Record]) -> Vec<String> {
    role_problems_of(record, all.iter().filter(|o| o.entry == record.entry))
}

/// [`role_problems`] against the records of the same entry only.
fn role_problems_of<'a>(record: &Record, same_entry: impl Iterator<Item = &'a Record>) -> Vec<String> {
    let mut out = Vec::new();
    if record.spec_agent == record.implementer {
        out.push(format!("{}: {} is both spec agent and implementer", record.entry, record.spec_agent));
    }
    for other in same_entry {
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
        files.push(entry.test_file(&rel.to_string_lossy()));
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

/// Write a hand-off record, as a new file under [`DIR`], and return it.
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
    // The ids are in a file name: letters, digits, '.', '_' and '-' only.
    for agent in [req.spec_agent, req.implementer] {
        if agent.is_empty() || !agent.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) {
            return Err(format!("agent id {agent:?} must be non-empty, of letters, digits, '.', '_' and '-' only"));
        }
    }
    let (existing, _) = load(repo)?;
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
    let path = repo.join(DIR).join(record.file_name());
    if let Some(dir) = path.parent() {
        util::create_dir(dir)?;
    }
    // `create_new`: never overwrite a record (the same two agents and entry within one second).
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| {
        format!("{}: {e} (a record of this hand-off in this second exists; run it again)", path.display())
    })?;
    std::io::Write::write_all(&mut file, record.file_text().as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(record)
}

/// The latest hand-off of `entry`, in the files or the legacy log.
pub fn latest(repo: &Path, entry: &str) -> Result<Record> {
    load(repo)?
        .0
        .into_iter()
        .rev()
        .find(|r| r.entry == entry)
        .ok_or_else(|| format!("no hand-off of {entry} in {DIR} or {LOG}"))
}

/// Write the files of the latest hand-off of `entry`, as committed at its
/// revision, into `out`: everything an implementer agent receives.
#[cfg(test)]
pub fn bundle(repo: &Path, entry: &str, out: &Path, heldout: Option<&Path>) -> Result<Record> {
    Ok(bundle_many(repo, &[entry], out, heldout)?.remove(0))
}

/// [`bundle`] for several entries handed over together (one implementer
/// gets them in one directory; ADR 0098's three C++ exception entries).
pub fn bundle_many(repo: &Path, entries: &[&str], out: &Path, heldout: Option<&Path>) -> Result<Vec<Record>> {
    if let Some(h) = heldout
        && (util::is_inside(out, h) || util::is_inside(h, out))
    {
        return Err(format!("{} overlaps the held-out store", out.display()));
    }
    if out.exists() && !util::files_under(out)?.is_empty() {
        return Err(format!("{} is not empty", out.display()));
    }
    let records = entries.iter().map(|e| latest(repo, e)).collect::<Result<Vec<_>>>()?;
    let mut seen = std::collections::BTreeSet::new();
    for f in records.iter().flat_map(|r| &r.files) {
        if !seen.insert(&f.path) {
            return Err(format!("{} is in more than one of the hand-offs", f.path));
        }
    }
    for record in &records {
        write_files(repo, record, out)?;
    }
    Ok(records)
}

fn write_files(repo: &Path, record: &Record, out: &Path) -> Result<()> {
    let requests: Vec<String> = record.files.iter().map(|f| format!("{}:{}", record.revision, f.path)).collect();
    let mut failure = None;
    util::git_blobs(repo, &requests, |i, data| {
        let f = &record.files[i];
        let result = match data {
            None => Err(format!("{} is not in git at {}", f.path, record.revision)),
            Some(data) if sha256::hex(&data) != f.sha256 => {
                Err(format!("{} at {} does not match the logged hash", f.path, record.revision))
            }
            Some(data) => util::write(&out.join(&f.path), data),
        };
        if let Err(e) = result {
            failure.get_or_insert(e);
        }
    })?;
    failure.map_or(Ok(()), Err)
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
    let (all, name_problems) = load(repo)?;
    let records: Vec<&Record> = all.iter().filter(|r| req.entry.is_none_or(|e| r.entry == e)).collect();
    let mut problems: Vec<String> =
        name_problems.into_iter().filter(|(entry, _)| req.entry.is_none_or(|e| e == entry)).map(|(_, p)| p).collect();
    if let Some(e) = req.entry
        && records.is_empty()
    {
        problems.push(format!("no hand-off of {e} in {DIR} or {LOG}"));
    }
    let held: BTreeMap<String, String> = match req.heldout {
        Some(h) => store::held_out_hashes(h)?,
        None => BTreeMap::new(),
    };
    // The roles, against the records of the same entry (a map: the log has thousands).
    let mut by_entry: BTreeMap<&str, Vec<&Record>> = BTreeMap::new();
    for r in &all {
        by_entry.entry(r.entry.as_str()).or_default().push(r);
    }
    // Every file of every record at its revision, read once: the hashes of
    // the distinct `revision:path` pairs, from one `git cat-file --batch`.
    let mut wanted: Vec<String> =
        records.iter().flat_map(|r| r.files.iter().map(|f| format!("{}:{}", r.revision, f.path))).collect();
    wanted.sort();
    wanted.dedup();
    let mut hashes: Vec<Option<String>> = vec![None; wanted.len()];
    util::git_blobs(repo, &wanted, |i, data| hashes[i] = data.map(|d| sha256::hex(&d)))?;
    let found: BTreeMap<&str, &Option<String>> = wanted.iter().map(String::as_str).zip(&hashes).collect();
    for r in &records {
        problems.extend(role_problems_of(r, by_entry[r.entry.as_str()].iter().copied()));
        for f in &r.files {
            match found[format!("{}:{}", r.revision, f.path).as_str()] {
                Some(hash) if *hash == f.sha256 => {}
                Some(_) => {
                    problems.push(format!("{}: {} at {} differs from the logged hash", r.entry, f.path, r.revision))
                }
                None => problems.push(format!("{}: {} is not in git at {}", r.entry, f.path, r.revision)),
            }
            if let Some(h) = held.get(&f.sha256) {
                problems.push(format!("{}: handed-over file {} is held-out test {h}", r.entry, f.path));
            }
        }
    }
    // Nothing committed under specs/ may be a held-out test.
    if !held.is_empty() {
        let files = util::files_under(&repo.join("specs"))?;
        let hashes = util::parallel_map(&files, util::default_jobs(8), |rel| {
            util::read(&repo.join("specs").join(rel)).map(|data| sha256::hex(&data))
        });
        for (rel, hash) in files.iter().zip(hashes) {
            if let Some(h) = held.get(&hash?) {
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
        // A second bundle into a directory that is not empty is refused;
        // the same entry twice in one bundle too (its files would collide).
        assert!(bundle(&repo, &e.id(), &out, Some(&heldout)).unwrap_err().contains("not empty"));
        let id = e.id();
        let twice = bundle_many(&repo, &[id.as_str(), id.as_str()], &base.join("b2"), Some(&heldout)).unwrap_err();
        assert!(twice.contains("more than one"), "{twice}");
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
        let r = record(&repo, &rec(&e, &heldout, "s", "i")).unwrap();
        // Tamper with the record: the hash of the spec no longer matches git.
        let path = repo.join(DIR).join(r.file_name());
        let text = util::read_string(&path).unwrap();
        let spec_hash = sha256::hex(&util::read(&repo.join(e.spec_path())).unwrap());
        util::write(&path, text.replace(&spec_hash, &"0".repeat(64))).unwrap();
        let problems = verify(&repo, &VerifyRequest::default()).unwrap();
        assert!(problems.iter().any(|p| p.contains("differs from the logged hash")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn sample() -> Record {
        Record {
            date: "2026-10-20 10:00:00".into(),
            entry: "linux/syscall/x86_64/write".into(),
            revision: "abc".into(),
            spec_agent: "spec-1".into(),
            implementer: "impl-1".into(),
            files: vec![
                FileHash { path: "specs/a.yaml".into(), sha256: "0".repeat(64) },
                FileHash { path: "specs/a.tests/t".into(), sha256: "f".repeat(64) },
            ],
        }
    }

    #[test]
    fn record_files_roundtrip() {
        let r = sample();
        let name = r.file_name();
        assert!(name.starts_with("20261020T100000Z-spec-1-impl-1-") && name.ends_with(".md"), "{name}");
        assert_eq!(parse_file(&r.file_text()).unwrap(), r);
        assert!(parse_file("Date (UTC): x\n").is_err());
        assert!(parse_file(&r.file_text().replace("Entry id:", "Entry:")).is_err());
        assert!(parse_file(&r.file_text().replace("sha256:ff", "sha256:f")).is_err());
        // The same entry in the same second gets the same name; another entry another one.
        let other = Record { entry: "linux/syscall/x86_64/read".into(), ..sample() };
        assert_ne!(other.file_name(), name);
    }

    #[test]
    fn record_writes_one_file_and_leaves_the_legacy_log_alone() {
        let (base, repo, heldout) = setup("onefile");
        let e = entry();
        let before = util::read_string(&repo.join(LOG)).unwrap();
        let r = record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap();
        assert_eq!(util::read_string(&repo.join(LOG)).unwrap(), before);
        let files = util::files_under(&repo.join(DIR)).unwrap();
        assert_eq!(files, [PathBuf::from(r.file_name())]);
        // An agent id that cannot be in a file name is refused.
        assert!(record(&repo, &rec(&e, &heldout, "spec/1", "impl-1")).is_err());
        assert!(record(&repo, &rec(&e, &heldout, "spec-1", "impl 1")).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_rows_and_files_are_checked_alike() {
        let (base, repo, heldout) = setup("both");
        let e = entry();
        // A legacy row for the entry: spec-1 handed it to impl-1 at a real revision.
        let head = util::git_text(&repo, &["rev-parse", "HEAD"]).unwrap();
        let files = handoff_files(&repo, &e).unwrap();
        let hashes: Vec<FileHash> = files
            .iter()
            .map(|p| FileHash {
                path: p.to_string_lossy().into_owned(),
                sha256: sha256::hex(&util::read(&repo.join(p)).unwrap()),
            })
            .collect();
        let legacy = Record { entry: e.id(), revision: head, files: hashes, ..sample() };
        let mut log = util::read_string(&repo.join(LOG)).unwrap();
        log.push_str(&legacy.row());
        log.push('\n');
        util::write(&repo.join(LOG), &log).unwrap();
        assert_eq!(verify(&repo, &VerifyRequest::default()).unwrap(), Vec::<String>::new());
        // The roles are tracked across both: impl-1 cannot become the spec agent in a new file.
        let err = record(&repo, &rec(&e, &heldout, "impl-1", "impl-2")).unwrap_err();
        assert!(err.contains("was an implementer"), "{err}");
        let new = record(&repo, &rec(&e, &heldout, "spec-1", "impl-2")).unwrap();
        // `latest` is the newest: the file, after the legacy row.
        assert_eq!(latest(&repo, &e.id()).unwrap(), new);
        assert_eq!(verify(&repo, &VerifyRequest::default()).unwrap(), Vec::<String>::new());
        // A file saying the legacy implementer wrote the spec is caught by `verify` too.
        let bad = Record { spec_agent: "impl-1".into(), implementer: "impl-3".into(), ..new.clone() };
        util::write(&repo.join(DIR).join(bad.file_name()), bad.file_text()).unwrap();
        let problems = verify(&repo, &VerifyRequest::default()).unwrap();
        assert!(problems.iter().any(|p| p.contains("was an implementer of it")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_sees_a_wrongly_named_or_unreadable_record_file() {
        let (base, repo, heldout) = setup("names");
        let e = entry();
        let r = record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap();
        // A README in the directory is not a record.
        util::write(&repo.join(DIR).join("README.md"), "about this directory").unwrap();
        assert_eq!(verify(&repo, &VerifyRequest::default()).unwrap(), Vec::<String>::new());
        let from = repo.join(DIR).join(r.file_name());
        std::fs::rename(&from, repo.join(DIR).join("renamed.md")).unwrap();
        let problems = verify(&repo, &VerifyRequest::default()).unwrap();
        assert!(problems.iter().any(|p| p.contains("file name does not match")), "{problems:?}");
        // ... which another entry's check does not report.
        let other = VerifyRequest { entry: Some("linux/syscall/x86_64/read"), ..Default::default() };
        assert_eq!(
            verify(&repo, &other).unwrap(),
            ["no hand-off of linux/syscall/x86_64/read in specs/handoffs or specs/HANDOFF_LOG.md"]
        );
        util::write(&repo.join(DIR).join("renamed.md"), "garbage").unwrap();
        assert!(verify(&repo, &VerifyRequest::default()).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_missing_at_the_revision_is_reported() {
        let (base, repo, heldout) = setup("missing");
        let e = entry();
        let r = record(&repo, &rec(&e, &heldout, "spec-1", "impl-1")).unwrap();
        let text = util::read_string(&repo.join(DIR).join(r.file_name())).unwrap();
        let gone = text.replace("write.tests/t0.c", "write.tests/nope.c").replace(&r.revision, &"1".repeat(40));
        util::write(&repo.join(DIR).join(r.file_name()), gone).unwrap();
        let problems = verify(&repo, &VerifyRequest::default()).unwrap();
        assert!(problems.iter().any(|p| p.contains("is not in git at")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
