//! What the scheduler keeps on disk (ADR 0226, section 3): each task's registered XML as a file under
//! `C:\Windows\System32\Tasks` (a folder is a directory), and one file of Poly's own,
//! [`STATE_FILE`], with what the XML does not hold: one line per task, tab-separated, `-` for
//! nothing:
//!
//! `path  enabled  last_run  last_result  missed  handled_until  boot  logon  registered  logon_boot`

use crate::sched::Events;
use crate::store::Task;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The directory of the task files.
pub const TASKS_DIR: &str = r"C:\Windows\System32\Tasks";
/// Poly's own state file (a dot name is not a task or folder name the store accepts from a path,
/// and the loader skips it).
pub const STATE_FILE: &str = r"C:\Windows\System32\Tasks\.poly-state";

/// One task's state as stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskState {
    pub path: String,
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub last_result: i32,
    pub missed: i32,
    pub handled_until: i64,
    pub events: Events,
    pub logon_boot: i64,
}

fn o(v: Option<i64>) -> String {
    v.map_or_else(|| String::from("-"), |v| format!("{v}"))
}

fn p(s: &str) -> Option<Option<i64>> {
    if s == "-" { Some(None) } else { s.parse().ok().map(Some) }
}

pub fn state_of(path: &str, t: &Task) -> TaskState {
    TaskState {
        path: String::from(path),
        enabled: t.enabled,
        last_run: t.last_run,
        last_result: t.last_result,
        missed: t.missed,
        handled_until: t.handled_until,
        events: t.events,
        logon_boot: t.logon_boot,
    }
}

pub fn apply(t: &mut Task, s: &TaskState) {
    t.enabled = s.enabled;
    t.last_run = s.last_run;
    t.last_result = s.last_result;
    t.missed = s.missed;
    t.handled_until = s.handled_until;
    t.events = s.events;
    t.logon_boot = s.logon_boot;
}

pub fn encode(states: &[TaskState]) -> String {
    let mut out = String::new();
    for s in states {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            s.path,
            u8::from(s.enabled),
            o(s.last_run),
            s.last_result,
            s.missed,
            s.handled_until,
            o(s.events.boot),
            o(s.events.logon),
            o(s.events.registered),
            s.logon_boot
        ));
    }
    out
}

/// The lines that read; a broken line is skipped.
pub fn decode(text: &str) -> Vec<TaskState> {
    let mut v = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 10 {
            continue;
        }
        let parsed = (|| {
            Some(TaskState {
                path: String::from(f[0]),
                enabled: f[1] == "1",
                last_run: p(f[2])?,
                last_result: f[3].parse().ok()?,
                missed: f[4].parse().ok()?,
                handled_until: f[5].parse().ok()?,
                events: Events { boot: p(f[6])?, logon: p(f[7])?, registered: p(f[8])? },
                logon_boot: f[9].parse().ok()?,
            })
        })();
        if let Some(s) = parsed {
            v.push(s);
        }
    }
    v
}

/// The file of a task path (`\a\b` → `C:\Windows\System32\Tasks\a\b`).
pub fn file_of(path: &str) -> String {
    let mut s = String::from(TASKS_DIR);
    if path != "\\" {
        s.push_str(path);
    }
    s
}
