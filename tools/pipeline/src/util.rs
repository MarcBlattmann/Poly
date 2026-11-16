//! Small helpers shared by the commands: argument parsing, UTC timestamps,
//! files, JSON and git.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T = ()> = std::result::Result<T, String>;

/// The error of a command (or part of one) that needs a Linux host: the
/// ptrace tracer, the diff runner's reference runs and the app/rootfs
/// builder use ptrace, namespaces, `/proc` and Unix file modes. On a Windows
/// host the tool still builds and says this instead (ADR 0124).
pub fn needs_linux(what: &str) -> String {
    format!("`pipeline {what}` needs a Linux host (ADR 0124); run it through polyd")
}

/// Command-line arguments: positionals, `--flag` switches and `--name value`
/// options (repeatable).
#[derive(Debug, Default)]
pub struct Args {
    pub positional: Vec<String>,
    options: BTreeMap<String, Vec<String>>,
    switches: Vec<String>,
}

impl Args {
    /// Parse `args`; the names in `switches` take no value, every other
    /// `--name` takes the next argument.
    pub fn parse(args: &[String], switches: &[&str]) -> Result<Args> {
        let mut out = Args::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if switches.contains(&name) {
                    out.switches.push(name.to_string());
                } else {
                    let value = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                    out.options.entry(name.to_string()).or_default().push(value.clone());
                }
            } else {
                out.positional.push(a.clone());
            }
        }
        Ok(out)
    }

    pub fn has(&self, switch: &str) -> bool {
        self.switches.iter().any(|s| s == switch)
    }

    pub fn opt(&self, name: &str) -> Option<&str> {
        self.options.get(name).and_then(|v| v.last()).map(String::as_str)
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.options.get(name).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default()
    }

    pub fn require(&self, name: &str) -> Result<&str> {
        self.opt(name).ok_or_else(|| format!("--{name} is required"))
    }

    /// Fail on options this command does not know.
    pub fn only(&self, known: &[&str]) -> Result {
        for name in self.options.keys().chain(&self.switches) {
            if !known.contains(&name.as_str()) {
                return Err(format!("unknown option --{name}"));
            }
        }
        Ok(())
    }
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian
/// calendar (the standard era-based conversion).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `secs` since the epoch as `YYYY-MM-DD HH:MM:SS` (UTC).
pub fn format_utc(secs: u64) -> String {
    let (y, mo, d) = civil_from_days((secs / 86_400) as i64);
    let t = secs % 86_400;
    format!("{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Now, as `YYYY-MM-DD HH:MM:SS` UTC.
pub fn now_utc() -> String {
    format_utc(now_secs())
}

/// Now, as a file-name-safe stamp `YYYYMMDD-HHMMSS`.
pub fn now_stamp() -> String {
    now_utc().replace(['-', ':'], "").replace(' ', "-")
}

pub fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_string(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write(path: &Path, data: impl AsRef<[u8]>) -> Result {
    if let Some(dir) = path.parent() {
        create_dir(dir)?;
    }
    std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn create_dir(dir: &Path) -> Result {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result {
    write(path, serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n")
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))
}

/// A relative `path` with `/` between its components, on every host.
pub fn slashed(path: &Path) -> PathBuf {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    PathBuf::from(parts.join("/"))
}

/// Every regular file under `dir`, as paths relative to it, sorted. The
/// separator is `/` on every host: the paths are recorded in manifests and
/// handed to `git show`, which only know `/` (a Windows host would give `\`).
pub fn files_under(dir: &Path) -> Result<Vec<PathBuf>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                walk(base, &path, out)?;
            } else if kind.is_file() {
                out.push(slashed(path.strip_prefix(base).unwrap()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if dir.is_dir() {
        walk(dir, dir, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// `path` made absolute and free of `.`/`..`, following symlinks of the
/// part that exists (so a symlink cannot hide that a path is inside another).
pub fn absolute(path: &Path) -> PathBuf {
    let joined =
        if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    // `.` and `..` first, lexically.
    let mut abs = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                abs.pop();
            }
            Component::CurDir => {}
            c => abs.push(c),
        }
    }
    // Canonicalize the longest existing prefix, then append the rest.
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => break,
        }
    }
    let mut out = existing.canonicalize().unwrap_or_else(|_| existing.to_path_buf());
    out.extend(rest.into_iter().rev());
    out
}

/// Whether `path` is `dir` or inside it (both made absolute first).
pub fn is_inside(path: &Path, dir: &Path) -> bool {
    absolute(path).starts_with(absolute(dir))
}

/// Run git in `repo` and return its stdout.
pub fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e} (is git installed?)"))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// The blobs named by `requests` (`<revision>:<path>`, as `git show` takes
/// them), read through ONE `git cat-file --batch` process instead of one
/// `git show` each: a process start costs milliseconds on a host and tens of
/// milliseconds in a container that reads the git directory over a network
/// file system (9p), and a hand-off verification reads thousands of files.
/// `each` gets the index of the request and the blob's bytes, or `None`
/// where the revision or the path does not exist (or is not a file). It runs
/// in the order of `requests`, one blob at a time, so the caller can hash
/// and drop it instead of keeping every file in memory. A request must not
/// contain a newline.
pub fn git_blobs(repo: &Path, requests: &[String], mut each: impl FnMut(usize, Option<Vec<u8>>)) -> Result {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;
    if requests.is_empty() {
        return Ok(());
    }
    if let Some(bad) = requests.iter().find(|r| r.contains(['\n', '\r'])) {
        return Err(format!("git cat-file: {bad:?} contains a line break"));
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git: {e} (is git installed?)"))?;
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let mut out = BufReader::new(child.stdout.take().expect("stdout was piped"));
    let result = std::thread::scope(|scope| {
        // Feed the requests from a thread: git answers while it reads, and
        // writing everything first would block once the pipes are full.
        scope.spawn(move || {
            let mut input = std::io::BufWriter::new(&mut stdin);
            for r in requests {
                if input.write_all(r.as_bytes()).and_then(|()| input.write_all(b"\n")).is_err() {
                    break;
                }
            }
            let _ = input.flush();
        });
        let mut read_all = || -> Result {
            let mut header = String::new();
            for i in 0..requests.len() {
                header.clear();
                let got = out.read_line(&mut header).map_err(|e| format!("git cat-file: {e}"))?;
                if got == 0 {
                    return Err("git cat-file ended before answering every request".to_string());
                }
                // `<oid> <type> <size>`, or `<request> missing` / `<request> ambiguous`.
                let mut words = header.trim_end().rsplitn(3, ' ');
                let (size, kind) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
                let Ok(size) = size.parse::<usize>() else {
                    each(i, None);
                    continue;
                };
                let mut data = vec![0u8; size + 1];
                out.read_exact(&mut data).map_err(|e| format!("git cat-file: {e}"))?;
                data.pop(); // the line feed after the contents
                each(i, (kind == "blob").then_some(data));
            }
            Ok(())
        };
        let r = read_all();
        // The feeder thread is joined when the scope ends: stop git first, or
        // it could wait for us to read while we wait for the feeder.
        if r.is_err() {
            let _ = child.kill();
        }
        r
    });
    let _ = child.wait();
    result
}

pub fn git_text(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(repo, args)?).trim().to_string())
}

/// `program` found on `PATH`, if it is there.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| p.is_file())
}

/// `f` over every item on up to `jobs` threads (at least one); the results
/// come back in the items' order. Threads take the next item when they
/// finish one, so a slow item does not hold up the rest.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, items.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                }
            });
        }
    });
    results.into_iter().map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()).expect("every item ran")).collect()
}

/// A default for `--jobs`: the machine's cores, at most `max`.
pub fn default_jobs(max: usize) -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, max.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_map_keeps_the_order() {
        let items: Vec<u64> = (0..50).collect();
        for jobs in [0, 1, 3, 64] {
            let out = parallel_map(&items, jobs, |i| {
                std::thread::sleep(std::time::Duration::from_micros((50 - i) * 20));
                i * 2
            });
            assert_eq!(out, items.iter().map(|i| i * 2).collect::<Vec<_>>());
        }
        assert!(parallel_map(&[] as &[u8], 4, |b| *b).is_empty());
        assert!((1..=4).contains(&default_jobs(4)));
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00");
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(format_utc(1_792_454_400 + 3661), "2026-10-20 01:01:01");
    }

    #[test]
    fn args_parse_options_switches_and_positionals() {
        let raw: Vec<String> = ["a", "--x", "1", "--flag", "b", "--x", "2"].iter().map(|s| s.to_string()).collect();
        let a = Args::parse(&raw, &["flag"]).unwrap();
        assert_eq!(a.positional, ["a", "b"]);
        assert_eq!(a.opt("x"), Some("2"));
        assert_eq!(a.all("x"), ["1", "2"]);
        assert!(a.has("flag"));
        assert!(a.only(&["x", "flag"]).is_ok());
        assert!(a.only(&["x"]).is_err());
        assert!(Args::parse(&["--x".to_string()], &[]).is_err());
    }

    #[test]
    fn inside_resolves_dots() {
        assert!(is_inside(Path::new("/a/b/../b/c"), Path::new("/a/b")));
        assert!(!is_inside(Path::new("/a/b/../c"), Path::new("/a/b")));
    }

    #[test]
    fn git_blobs_match_git_show() {
        let dir = std::env::temp_dir().join(format!("poly-util-blobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        write(&dir.join("big.bin"), &big).unwrap();
        write(&dir.join("empty"), b"").unwrap();
        write(&dir.join("sub/a b.txt"), b"two words\r\nand a CRLF\n").unwrap();
        for args in [
            &["init", "-q"][..],
            &["add", "."],
            &["-c", "user.name=t", "-c", "user.email=t@e.x", "commit", "-q", "-m", "x"],
        ] {
            git(&dir, args).unwrap();
        }
        let requests: Vec<String> =
            ["HEAD:big.bin", "HEAD:empty", "HEAD:nope", "HEAD:sub/a b.txt", "HEAD:sub", "0000000:x", "HEAD:big.bin"]
                .map(String::from)
                .to_vec();
        let mut got = vec![None; requests.len()];
        git_blobs(&dir, &requests, |i, data| got[i] = data).unwrap();
        for (r, g) in requests.iter().zip(&got) {
            // A directory is not a file for us (`git show` would list it).
            let expected = if r == "HEAD:sub" { None } else { git(&dir, &["show", r]).ok() };
            assert_eq!(*g, expected, "{r}");
        }
        assert!(git_blobs(&dir, &["HEAD:a\nb".to_string()], |_, _| {}).is_err());
        assert!(git_blobs(&dir.join("not-a-repo"), &["HEAD:x".to_string()], |_, _| {}).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
