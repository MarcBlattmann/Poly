//! Small helpers shared by the commands: argument parsing, UTC timestamps,
//! files, JSON and git.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T = ()> = std::result::Result<T, String>;

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

/// (year, month, day) to days since 1970-01-01: the inverse of
/// [`civil_from_days`].
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD HH:MM:SS` (UTC, as [`format_utc`] writes it) as seconds
/// since the epoch; `None` if it is not in that form.
pub fn parse_utc(text: &str) -> Option<u64> {
    let (date, time) = text.trim().split_once(' ')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<u32>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.splitn(3, ':').map(|p| p.parse::<u64>().ok());
    let (h, mi, s) = (t.next()??, t.next()??, t.next()??);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let days = u64::try_from(days_from_civil(i64::from(y), mo, da)).ok()?;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
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

/// Every regular file under `dir`, as paths relative to it, sorted.
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
                out.push(path.strip_prefix(base).unwrap().to_path_buf());
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

pub fn git_text(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(repo, args)?).trim().to_string())
}

/// `program` found on `PATH`, if it is there.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| p.is_file())
}

/// Quote `s` for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
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
        for secs in [0, 951_782_400, 951_868_799, 1_792_454_400 + 3661, 4_102_444_800] {
            assert_eq!(parse_utc(&format_utc(secs)), Some(secs));
        }
        assert_eq!(parse_utc("2026-13-01 00:00:00"), None);
        assert_eq!(parse_utc("yesterday"), None);
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
    fn quoting() {
        assert_eq!(shell_quote("plain/path.txt"), "plain/path.txt");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }
}
