//! What the dashboard shows, read from the loop's state directory, its log
//! and `/proc`, without changing anything: no file is created or written,
//! no lock is held (the loop's `flock` locks are read from `/proc/locks`;
//! only where that is unreadable is a lock probed, for an instant, on a
//! read-only descriptor).
//!
//! State files are replaced by an atomic rename (`state::write_atomic`),
//! so a read sees the old or the new file; the temporary files of a write
//! in progress (`<key>.tmp.<pid>`) are skipped as `loop status` skips them,
//! and a file that does not parse is reported, not fatal.

use super::super::cycle;
use super::super::state::{EntryState, Pause, Step};
use crate::util;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Where to look; tests point every path into a temporary directory.
#[derive(Clone, Debug)]
pub struct Sources {
    pub state_dir: PathBuf,
    pub log_file: Option<PathBuf>,
    /// `/proc` (process list, `locks`, `loadavg`, `meminfo`, `stat`).
    pub proc_dir: PathBuf,
    /// `/etc/passwd`, for user names.
    pub passwd: PathBuf,
    /// The file system whose free space is shown.
    pub disk_path: PathBuf,
    /// The systemd unit to ask `systemctl is-active` about.
    pub service: Option<String>,
    pub host: String,
    pub config: Option<PathBuf>,
    /// `sysconf(_SC_CLK_TCK)`: the unit of start times in `/proc/<pid>/stat`.
    pub clock_ticks: u64,
    /// Lines of the pipeline log to show.
    pub log_lines: usize,
}

/// The configuration values the dashboard shows or needs.
#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub max_parallel: usize,
    pub max_cycles: u32,
    pub max_errors: u32,
    pub spec_cycles: u32,
    pub poll_secs: u64,
    /// `<os>/<layer>/`: left out of entry ids on screen.
    pub id_prefix: String,
}

impl Limits {
    pub fn of(cfg: &super::super::config::Config) -> Limits {
        Limits {
            max_parallel: cfg.max_parallel,
            max_cycles: cfg.max_cycles,
            max_errors: cfg.max_errors,
            spec_cycles: cfg.spec_cycles,
            poll_secs: cfg.poll_secs,
            id_prefix: format!("{}/{}/", cfg.os, cfg.layer),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Space {
    pub free: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Disk {
    pub path: String,
    #[serde(flatten)]
    pub space: Space,
}

/// A process from `/proc`.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Seconds since the epoch.
    pub started: u64,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RolePause {
    pub role: String,
    pub until: u64,
    pub reason: String,
    /// Still in force now.
    pub active: bool,
}

/// What an entry that is not being driven right now waits for.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Wait {
    pub what: String,
    pub until: u64,
}

/// One entry, as a row of the table.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Row {
    pub entry: String,
    /// The state's step (`awaiting-merge`, ...).
    pub step: String,
    /// The step as shown: claim, spec, spec-check, handoff, implement,
    /// verify, publish, waiting (the PR waits for its merge), refresh,
    /// human, done.
    pub stage: String,
    pub cycle: u32,
    pub spec_cycle: u32,
    pub errors: u32,
    /// When the entry reached its current step (seconds since the epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// A worker holds the entry's lock.
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Why it is in the human queue; else the last result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<Wait>,
}

/// An agent process (Claude Code, OpenCode) under a role session.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Agent {
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub started: u64,
    pub program: String,
}

/// Something a worker is doing right now.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Activity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// "implementer session", "CI", "diff run", "git fetch", ...
    pub what: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Agent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    /// Seconds since the command's log was last written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_age: Option<u64>,
    /// Seen in the process list; else guessed from the newest command log.
    pub exact: bool,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Totals {
    /// Merged (or pushed) today, UTC.
    pub done_today: usize,
    pub done: usize,
    pub human: usize,
    /// PRs the state knows to be open (waiting for the merge, or being refreshed).
    pub open_prs: usize,
    /// Entries in progress, not counting PRs waiting for the merge (what
    /// `max_parallel` limits).
    pub in_flight: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub time: u64,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    pub state_dir: String,
    pub limits: Limits,
    /// `systemctl is-active` (active, inactive, failed, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// `pipeline loop` processes.
    pub loops: Vec<Proc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<Space>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<Disk>,
    pub pauses: Vec<RolePause>,
    /// Roles with an agent session (their lock held).
    pub roles_busy: Vec<String>,
    pub entries: Vec<Row>,
    pub activity: Vec<Activity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    pub log_tail: Vec<String>,
    pub totals: Totals,
    /// Files that could not be read, and the like.
    pub problems: Vec<String>,
}

/// Merged entries older than this are not listed (only counted).
const DONE_SHOWN_SECS: u64 = 24 * 3600;
/// How much of the end of the pipeline log is read.
const LOG_TAIL_BYTES: u64 = 64 * 1024;

pub fn gather(src: &Sources, limits: &Limits, now: u64) -> Snapshot {
    let mut problems = Vec::new();
    let (states, bad) = read_states(&src.state_dir.join("entries"));
    problems.extend(bad);

    let users = read_users(&src.passwd);
    let procs = read_procs(&src.proc_dir, src.clock_ticks, &users);
    let loops: Vec<Proc> = procs.values().filter(|p| is_loop(&p.argv)).cloned().collect();
    let locks = Locks::read(&src.proc_dir, &src.state_dir.join("locks"), &loops);

    let tail = read_tail(src.log_file.as_deref(), LOG_TAIL_BYTES);
    let workers = workers_from_log(&tail);
    let log_tail: Vec<String> = {
        let lines: Vec<&str> = tail.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(src.log_lines)..].iter().map(|l| l.to_string()).collect()
    };

    let pauses = read_pauses(&src.state_dir, now, &mut problems);
    let active_pause = |role: &str| pauses.iter().find(|p| p.role == role && p.active).map(|p| p.until);

    let mut rows = Vec::new();
    let mut totals = Totals::default();
    for st in &states {
        let since = step_since(st);
        let today = |t: Option<u64>| t.is_some_and(|t| t / 86_400 == now / 86_400);
        match st.step {
            Step::Merged => {
                totals.done += 1;
                totals.done_today += usize::from(today(since));
                if since.is_some_and(|t| now.saturating_sub(t) > DONE_SHOWN_SECS) {
                    continue;
                }
            }
            Step::Human => totals.human += 1,
            Step::AwaitingMerge | Step::Refresh if st.pr_url.is_some() => totals.open_prs += 1,
            _ => {}
        }
        if !st.step.terminal() && st.step != Step::AwaitingMerge {
            totals.in_flight += 1;
        }
        let running = locks.held(&format!("entry-{}", super::super::state::key(&st.entry)));
        let wait = if running || st.step.terminal() {
            None
        } else if st.retry_at > now {
            Some(Wait { what: "error back-off, retry".into(), until: st.retry_at })
        } else if let Some(role) = st.step.role()
            && !cycle::fetch_only(st)
            && let Some(until) = active_pause(role)
        {
            Some(Wait { what: format!("{role} role paused, resumes"), until })
        } else if st.step == Step::AwaitingMerge {
            Some(Wait { what: "next PR check".into(), until: st.last_poll + limits.poll_secs })
        } else {
            None
        };
        let reason = match st.step {
            Step::Human => st.reason.clone(),
            _ => st.last_result.clone(),
        };
        rows.push(Row {
            entry: st.entry.clone(),
            step: st.step.name().into(),
            stage: stage(st.step).into(),
            cycle: st.cycle,
            spec_cycle: st.spec_cycle,
            errors: st.errors,
            since,
            running,
            worker: workers.get(&st.entry).cloned(),
            pr_url: st.pr_url.clone(),
            reason,
            wait,
        });
    }
    // In flight first (running ones on top, oldest first), then the human
    // queue and what was done, newest first.
    let rank = |r: &Row| match r.stage.as_str() {
        "human" => 2,
        "done" => 3,
        _ if r.running => 0,
        _ => 1,
    };
    rows.sort_by(|a, b| {
        rank(a).cmp(&rank(b)).then_with(|| {
            if rank(a) >= 2 { b.since.cmp(&a.since) } else { a.since.cmp(&b.since) }.then_with(|| a.entry.cmp(&b.entry))
        })
    });

    let in_progress: Vec<&EntryState> = states.iter().filter(|s| !s.step.terminal()).collect();
    let logs = command_logs(&src.state_dir.join("logs"), &in_progress);
    let activity = activity(&procs, &loops, &rows, &in_progress, &logs, &workers, now);

    let roles_busy =
        ["spec", "implementer"].iter().filter(|r| locks.held(&format!("role-{r}"))).map(|r| r.to_string()).collect();

    let disk_path = existing_ancestor(&src.disk_path);
    Snapshot {
        time: now,
        host: src.host.clone(),
        config: src.config.as_ref().map(|p| p.display().to_string()),
        state_dir: src.state_dir.display().to_string(),
        limits: limits.clone(),
        service: src.service.as_deref().and_then(service_state),
        loops,
        load: read_load(&src.proc_dir),
        memory: read_memory(&src.proc_dir),
        disk: disk_path
            .as_deref()
            .and_then(|p| disk_space(p).map(|space| Disk { path: src.disk_path.display().to_string(), space })),
        pauses,
        roles_busy,
        entries: rows,
        activity,
        log_file: src.log_file.as_ref().map(|p| p.display().to_string()),
        log_tail,
        totals,
        problems,
    }
}

/// The step as the dashboard names it.
pub fn stage(step: Step) -> &'static str {
    match step {
        Step::AwaitingMerge => "waiting",
        Step::Merged => "done",
        s => s.name(),
    }
}

/// Every entry state, and a message for each file that would not parse.
pub fn read_states(dir: &Path) -> (Vec<EntryState>, Vec<String>) {
    let mut states = Vec::new();
    let mut bad = Vec::new();
    let names = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect::<Vec<_>>(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return (states, vec![format!("{}: {e}", dir.display())]),
    };
    for p in names {
        // `<key>.json` only: a write in progress is `<key>.tmp.<pid>`.
        if p.extension().is_none_or(|x| x != "json") || !p.is_file() {
            continue;
        }
        match util::read_json::<EntryState>(&p) {
            Ok(s) => states.push(s),
            // Gone between the listing and the read: the entry was reset.
            Err(_) if !p.exists() => {}
            Err(e) => bad.push(e),
        }
    }
    states.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.entry.cmp(&b.entry)));
    (states, bad)
}

/// When the entry reached its current step. A step's last note is written
/// under the step it leaves (`note`, then `step = next`), so the entry
/// arrived with the last event of another step; the human queue's note is
/// written under `human` itself.
pub fn step_since(st: &EntryState) -> Option<u64> {
    let h = &st.history;
    let first_here = h.iter().rposition(|e| e.step != st.step).map_or(0, |i| i + 1);
    let event = if st.step == Step::Human && first_here < h.len() {
        h.get(first_here)
    } else if first_here > 0 {
        h.get(first_here - 1)
    } else {
        None
    };
    event.and_then(|e| util::parse_utc(&e.time)).or_else(|| util::parse_utc(&st.created))
}

fn read_pauses(state_dir: &Path, now: u64, problems: &mut Vec<String>) -> Vec<RolePause> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(state_dir) else { return out };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for p in files {
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(role) = name.strip_prefix("pause-").and_then(|r| r.strip_suffix(".json")) else { continue };
        match util::read_json::<Pause>(&p) {
            Ok(pause) => out.push(RolePause {
                role: role.into(),
                until: pause.until,
                reason: pause.reason,
                active: pause.until > now,
            }),
            Err(_) if !p.exists() => {}
            Err(e) => problems.push(e),
        }
    }
    out
}

/// The last `bytes` of the file, from the first whole line on.
pub fn read_tail(path: Option<&Path>, bytes: u64) -> String {
    let Some(path) = path else { return String::new() };
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(bytes).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        // The first line is cut.
        return text.split_once('\n').map_or(String::new(), |(_, rest)| rest.to_string());
    }
    text
}

/// The worker that logged last about each entry: log lines are
/// `YYYY-MM-DD HH:MM:SS w0 <entry>: ...`.
pub fn workers_from_log(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let mut words = line.splitn(5, ' ');
        let (Some(_), Some(_), Some(worker), Some(entry)) = (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        let Some(entry) = entry.strip_suffix(':') else { continue };
        if worker.len() > 1 && worker.starts_with('w') && worker[1..].bytes().all(|b| b.is_ascii_digit()) {
            out.insert(entry.to_string(), worker.to_string());
        }
    }
    out
}

/// A command's log, `<state>/logs/<key>/<YYYYMMDD-HHMMSS>-<pid>-<n>-<what>.log`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandLog {
    pub entry: String,
    pub path: PathBuf,
    pub started: u64,
    pub pid: u32,
    pub n: u32,
    pub what: String,
    pub modified: Option<u64>,
}

pub fn parse_log_name(name: &str) -> Option<(u64, u32, u32, String)> {
    let stem = name.strip_suffix(".log")?;
    let mut parts = stem.splitn(5, '-');
    let (date, time, pid, n, what) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if date.len() != 8 || time.len() != 6 {
        return None;
    }
    let stamp = format!("{}-{}-{} {}:{}:{}", &date[..4], &date[4..6], &date[6..], &time[..2], &time[2..4], &time[4..]);
    Some((util::parse_utc(&stamp)?, pid.parse().ok()?, n.parse().ok()?, what.to_string()))
}

/// The command logs of the entries in progress, oldest first.
fn command_logs(logs_dir: &Path, entries: &[&EntryState]) -> Vec<CommandLog> {
    let mut out = Vec::new();
    for st in entries {
        let dir = logs_dir.join(super::super::state::key(&st.entry));
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some((started, pid, n, what)) = parse_log_name(&name) else { continue };
            let modified = e.metadata().ok().and_then(|m| u64::try_from(m.mtime()).ok());
            out.push(CommandLog { entry: st.entry.clone(), path: e.path(), started, pid, n, what, modified });
        }
    }
    out.sort_by_key(|a| (a.started, a.pid, a.n));
    out
}

/// What the workers do right now. With the loop in the process list: its
/// child processes, each matched to its entry (by an entry id among its
/// arguments, else by the command log it writes, which names the loop's
/// pid and the second it started). Without: for each entry whose lock is
/// held, its newest command log if nothing was noted in the state since.
fn activity(
    procs: &BTreeMap<u32, Proc>,
    loops: &[Proc],
    rows: &[Row],
    entries: &[&EntryState],
    logs: &[CommandLog],
    workers: &BTreeMap<String, String>,
    now: u64,
) -> Vec<Activity> {
    let mut out = Vec::new();
    let log_age = |l: &CommandLog| l.modified.map(|m| now.saturating_sub(m));
    let loop_pids: BTreeSet<u32> = loops.iter().map(|p| p.pid).collect();
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for p in procs.values() {
        children.entry(p.ppid).or_default().push(p.pid);
    }
    if !loops.is_empty() {
        let mut busy = BTreeSet::new();
        for child in procs.values().filter(|p| loop_pids.contains(&p.ppid)) {
            let mut entry = entries.iter().find(|s| child.argv.contains(&s.entry)).map(|s| s.entry.clone());
            // Its log names the same loop pid. For a known entry: the
            // entry's newest that started by then; else one that started
            // within a few seconds of it.
            let ours = logs.iter().filter(|l| l.pid == child.ppid && l.started <= child.started + 3);
            let log = match &entry {
                Some(e) => ours.filter(|l| l.entry == *e).max_by_key(|l| (l.started, l.n)),
                None => ours
                    .filter(|l| l.started.abs_diff(child.started) <= 3)
                    .min_by_key(|l| l.started.abs_diff(child.started)),
            };
            if entry.is_none() {
                entry = log.map(|l| l.entry.clone());
            }
            if let Some(e) = &entry {
                busy.insert(e.clone());
            }
            let agent = descendants(child.pid, &children)
                .into_iter()
                .filter_map(|pid| procs.get(&pid))
                .filter(|p| is_agent(&p.argv))
                .min_by_key(|p| p.started)
                .map(|p| Agent { pid: p.pid, user: p.user.clone(), started: p.started, program: base(&p.argv[0]) });
            out.push(Activity {
                worker: entry.as_ref().and_then(|e| workers.get(e).cloned()),
                entry,
                what: label(&child.argv),
                started: Some(child.started),
                pid: Some(child.pid),
                agent,
                log: log.map(|l| l.path.display().to_string()),
                log_age: log.and_then(log_age),
                exact: true,
            });
        }
        for r in rows.iter().filter(|r| r.running && !busy.contains(&r.entry)) {
            out.push(Activity {
                worker: r.worker.clone(),
                entry: Some(r.entry.clone()),
                what: format!("{} (between commands)", r.stage),
                started: None,
                pid: None,
                agent: None,
                log: None,
                log_age: None,
                exact: true,
            });
        }
    } else {
        for r in rows.iter().filter(|r| r.running) {
            let updated = entries.iter().find(|s| s.entry == r.entry).and_then(|s| util::parse_utc(&s.updated));
            let newest = logs.iter().rev().find(|l| l.entry == r.entry);
            let current = newest.filter(|l| updated.is_none_or(|u| l.started >= u));
            out.push(Activity {
                worker: r.worker.clone(),
                entry: Some(r.entry.clone()),
                what: current.map_or_else(|| format!("{} (between commands)", r.stage), |l| log_label(&l.what)),
                started: current.map(|l| l.started),
                pid: None,
                agent: None,
                log: current.map(|l| l.path.display().to_string()),
                log_age: current.and_then(log_age),
                exact: false,
            });
        }
    }
    out.sort_by(|a, b| {
        a.worker.cmp(&b.worker).then_with(|| a.entry.cmp(&b.entry)).then_with(|| a.started.cmp(&b.started))
    });
    out
}

fn descendants(pid: u32, children: &BTreeMap<u32, Vec<u32>>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut todo = vec![pid];
    while let Some(p) = todo.pop() {
        for &c in children.get(&p).into_iter().flatten() {
            if !out.contains(&c) && out.len() < 4096 {
                out.push(c);
                todo.push(c);
            }
        }
    }
    out
}

fn base(arg: &str) -> String {
    arg.rsplit('/').next().unwrap_or(arg).to_string()
}

/// `pipeline loop` (not `loop status`, `reset`, `watch` or a dry run).
pub fn is_loop(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| base(a) == "pipeline")
        && argv.get(1).is_some_and(|a| a == "loop")
        && !argv[2..].iter().any(|a| matches!(a.as_str(), "status" | "reset" | "watch" | "--dry-run"))
}

/// An LLM agent's process: Claude Code (native or on node) or OpenCode.
pub fn is_agent(argv: &[String]) -> bool {
    let Some(first) = argv.first() else { return false };
    match base(first).as_str() {
        "claude" | "opencode" => true,
        "node" | "bun" => argv.get(1).is_some_and(|a| a.contains("claude") || a.contains("opencode")),
        _ => false,
    }
}

/// What a command the loop runs is, from its arguments.
pub fn label(argv: &[String]) -> String {
    if let Some(i) = argv.iter().position(|a| base(a) == "poly-role") {
        let role = argv.get(i + 1).map_or("role", String::as_str);
        return match argv.get(i + 2).map(String::as_str) {
            Some("run") if role == "spec" => "spec agent session".into(),
            Some("run") => format!("{role} session"),
            Some("sync") => format!("{role} checkout sync"),
            Some(other) => format!("{role} {other}"),
            None => format!("{role} wrapper"),
        };
    }
    if let Some(i) = argv.iter().position(|a| a == "xtask") {
        let words: Vec<&str> =
            argv[i + 1..].iter().map(String::as_str).take_while(|a| !a.starts_with('-')).take(3).collect();
        return match words.as_slice() {
            ["ci", ..] => "CI".into(),
            ["pipeline", "diff", ..] => "diff run".into(),
            [] => "xtask".into(),
            w => w.join(" "),
        };
    }
    let Some(first) = argv.first() else { return "?".into() };
    let program = base(first);
    if program == "git" || program == "gh" {
        let mut i = 1;
        while i < argv.len() && matches!(argv[i].as_str(), "-C" | "-c") {
            i += 2;
        }
        let sub: Vec<&str> = argv
            .get(i..)
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .take(if program == "gh" { 2 } else { 1 })
            .collect();
        return format!("{program} {}", sub.join(" ")).trim_end().to_string();
    }
    program
}

/// What a command log's `what` part means.
pub fn log_label(what: &str) -> String {
    match what {
        "ci" => "CI".into(),
        "diff" => "diff run".into(),
        "spec" => "spec agent session".into(),
        "implementer" => "implementer session".into(),
        w => w.replace('-', " "),
    }
}

fn read_users(passwd: &Path) -> BTreeMap<u32, String> {
    let text = std::fs::read_to_string(passwd).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let uid = f.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

/// Every process whose `stat` and `cmdline` are readable (they are for
/// every process unless `/proc` is mounted with `hidepid`). Kernel threads
/// (an empty command line) are left out.
pub fn read_procs(proc_dir: &Path, ticks: u64, users: &BTreeMap<u32, String>) -> BTreeMap<u32, Proc> {
    let mut out = BTreeMap::new();
    let boot = std::fs::read_to_string(proc_dir.join("stat"))
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse::<u64>().ok()));
    let Ok(rd) = std::fs::read_dir(proc_dir) else { return out };
    for e in rd.filter_map(|e| e.ok()) {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let dir = e.path();
        let Ok(cmdline) = std::fs::read(dir.join("cmdline")) else { continue };
        let argv: Vec<String> = cmdline
            .split(|&b| b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        if argv.is_empty() {
            continue;
        }
        let Some((ppid, start_ticks)) = std::fs::read_to_string(dir.join("stat")).ok().and_then(|s| parse_stat(&s))
        else {
            continue;
        };
        let uid = std::fs::read_to_string(dir.join("status"))
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:")?.split_whitespace().next()?.parse().ok()))
            .unwrap_or(u32::MAX);
        let started = boot.map_or(0, |b| b + start_ticks / ticks.max(1));
        out.insert(pid, Proc { pid, ppid, uid, user: users.get(&uid).cloned(), started, argv });
    }
    out
}

/// (ppid, start time in clock ticks since boot) from `/proc/<pid>/stat`:
/// `pid (comm) state ppid ... starttime(22nd) ...`; `comm` may hold
/// spaces and parentheses, so fields count from its last `)`.
pub fn parse_stat(text: &str) -> Option<(u32, u64)> {
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some((f.get(1)?.parse().ok()?, f.get(19)?.parse().ok()?))
}

/// Which of the loop's lock files are held.
#[derive(Debug, Default)]
pub struct Locks {
    held: BTreeSet<String>,
}

impl Locks {
    /// From `/proc/locks`: a held `flock` names its holder's pid, the
    /// device (`major:minor` in hex) and the inode of the file. The device
    /// numbers of some file systems (overlayfs) differ from what `stat`
    /// says, so a lock of a `pipeline loop` process on the same inode
    /// counts too. Without `/proc/locks`, each lock file is probed: a
    /// non-blocking `flock` on a read-only descriptor, dropped at once.
    pub fn read(proc_dir: &Path, locks_dir: &Path, loops: &[Proc]) -> Locks {
        let mut out = Locks::default();
        let Ok(rd) = std::fs::read_dir(locks_dir) else { return out };
        let files: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_str()?.strip_suffix(".lock")?.to_string();
                Some((name, e.path()))
            })
            .collect();
        match std::fs::read_to_string(proc_dir.join("locks")) {
            Ok(text) => {
                let held = parse_proc_locks(&text);
                let loop_pids: BTreeSet<u32> = loops.iter().map(|p| p.pid).collect();
                for (name, path) in files {
                    let Ok(m) = std::fs::metadata(&path) else { continue };
                    let (major, minor) = dev_numbers(m.dev());
                    if held.iter().any(|l| {
                        l.inode == m.ino() && ((l.major, l.minor) == (major, minor) || loop_pids.contains(&l.pid))
                    }) {
                        out.held.insert(name);
                    }
                }
            }
            Err(_) => {
                for (name, path) in files {
                    let Ok(f) = std::fs::File::open(&path) else { continue };
                    if matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
                        out.held.insert(name);
                    }
                }
            }
        }
        out
    }

    pub fn held(&self, name: &str) -> bool {
        self.held.contains(name)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeldLock {
    pub pid: u32,
    pub major: u32,
    pub minor: u32,
    pub inode: u64,
}

/// The granted `FLOCK` locks of `/proc/locks` (`N: FLOCK ADVISORY WRITE
/// <pid> <maj>:<min>:<inode> 0 EOF`; waiters, `N: -> FLOCK ...`, are not
/// holders).
pub fn parse_proc_locks(text: &str) -> Vec<HeldLock> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.get(1) != Some(&"FLOCK") {
                return None;
            }
            let mut dev = f.get(5)?.split(':');
            Some(HeldLock {
                pid: f.get(4)?.parse().ok()?,
                major: u32::from_str_radix(dev.next()?, 16).ok()?,
                minor: u32::from_str_radix(dev.next()?, 16).ok()?,
                inode: dev.next()?.parse().ok()?,
            })
        })
        .collect()
}

/// glibc's `major()`/`minor()` of a `dev_t`.
pub fn dev_numbers(dev: u64) -> (u32, u32) {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    (major as u32, minor as u32)
}

fn read_load(proc_dir: &Path) -> Option<[f64; 3]> {
    let text = std::fs::read_to_string(proc_dir.join("loadavg")).ok()?;
    let mut f = text.split_whitespace().map(|v| v.parse::<f64>().ok());
    Some([f.next()??, f.next()??, f.next()??])
}

fn read_memory(proc_dir: &Path) -> Option<Space> {
    let text = std::fs::read_to_string(proc_dir.join("meminfo")).ok()?;
    let kb = |key: &str| {
        text.lines().find_map(|l| l.strip_prefix(key)?.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
    };
    Some(Space { free: kb("MemAvailable:")? * 1024, total: kb("MemTotal:")? * 1024 })
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

fn disk_space(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is plain integers; all zeroes is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path; statvfs(3) fills `st`.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let unit = st.f_frsize as u64;
    Some(Space { free: st.f_bavail as u64 * unit, total: st.f_blocks as u64 * unit })
}

/// `systemctl is-active <unit>`: its one word, even when it exits non-zero
/// (inactive, failed); `None` without systemctl or when it does not answer
/// within two seconds.
fn service_state(unit: &str) -> Option<String> {
    let mut child = std::process::Command::new("systemctl")
        .args(["is-active", unit])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    child.stdout.take()?.read_to_string(&mut text).ok()?;
    let word = text.trim();
    (!word.is_empty()).then(|| word.to_string())
}
