//! Build steps run at the same time, shown as a live table
//! (`cargo xtask start`, ADR 0133).
//!
//! Every build step of xtask goes through [`step`]. Normally that prints
//! `==> name` and lets the step write to the terminal, as before. Inside
//! [`show`] the steps (several at once: the kernel, the native programs,
//! the Win32 DLLs, the desktop's files) write to a log each under
//! `target/poly-start/`, and the terminal shows one row per step: running
//! (with its newest log line) or done, with the time it took. What xtask
//! itself would say meanwhile ([`note`]) is kept and printed after the
//! table, with the summary.

use crate::Result;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Whether [`show`] is drawing the table.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The steps so far (of the current [`show`]).
static ROWS: Mutex<Vec<Row>> = Mutex::new(Vec::new());
/// What was said while the table was up.
static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Debug)]
struct Row {
    name: String,
    started: Instant,
    /// When it ended and whether it succeeded.
    ended: Option<(Instant, bool)>,
    log: Option<PathBuf>,
}

fn rows() -> std::sync::MutexGuard<'static, Vec<Row>> {
    ROWS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Say `line` (`==> ` is added): on the terminal, or after the table.
pub fn note(line: &str) {
    if ACTIVE.load(Ordering::SeqCst) {
        NOTES.lock().unwrap_or_else(|e| e.into_inner()).push(format!("==> {line}"));
    } else {
        eprintln!("==> {line}");
    }
}

/// Run `f` as a step called `name` of the table (work done in xtask
/// itself, such as gathering a disk's files); just `f` without a table.
pub fn task<T>(name: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if !ACTIVE.load(Ordering::SeqCst) {
        return f();
    }
    let i = start_row(name, None);
    let r = f();
    end_row(i, r.is_ok());
    r
}

fn start_row(name: &str, log: Option<PathBuf>) -> usize {
    let mut rows = rows();
    rows.push(Row { name: name.to_string(), started: Instant::now(), ended: None, log });
    rows.len() - 1
}

fn end_row(i: usize, ok: bool) {
    if let Some(r) = rows().get_mut(i) {
        r.ended = Some((Instant::now(), ok));
    }
}

/// A file name for a step's log.
fn slug(name: &str) -> String {
    let mut s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').chars().take(60).collect()
}

/// Run the build step `cmd` called `name` (see the module docs).
pub fn step(name: &str, cmd: &mut Command) -> Result {
    if !ACTIVE.load(Ordering::SeqCst) {
        eprintln!("==> {name}");
        let status = cmd.status().map_err(|e| format!("{name}: {e}"))?;
        return if status.success() { Ok(()) } else { Err(format!("{name} failed ({status})")) };
    }
    let dir = crate::root().join("target").join("poly-start");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let log = dir.join(format!("{}.log", slug(name)));
    let file = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = file.try_clone().map_err(|e| format!("{}: {e}", log.display()))?;
    cmd.stdout(file).stderr(err).stdin(std::process::Stdio::null());
    let i = start_row(name, Some(log.clone()));
    let status = cmd.status();
    let ok = matches!(&status, Ok(s) if s.success());
    end_row(i, ok);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{name} failed ({s}); its output is in {}:\n{}", log.display(), tail(&log, 20))),
        Err(e) => Err(format!("{name}: {e}")),
    }
}

/// The last `n` lines of `path` (its last 16 KiB at most), without
/// terminal escapes.
fn tail(path: &Path, n: usize) -> String {
    let mut text = String::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = f.seek(SeekFrom::Start(len.saturating_sub(16 * 1024)));
        let mut bytes = Vec::new();
        let _ = f.read_to_end(&mut bytes);
        text = crate::console::strip_ansi(&String::from_utf8_lossy(&bytes));
    }
    let lines: Vec<&str> = text.split(['\n', '\r']).map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// `d` as the table shows it: `12.3 s`, `2 min 05 s`.
pub fn took(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 { format!("{s:.1} s") } else { format!("{} min {:02} s", d.as_secs() / 60, d.as_secs() % 60) }
}

/// One row of the table, at most `width` columns: a mark, the name, the
/// time, and while it runs its newest log line.
fn row_text(r: &Row, now: Instant, frame: usize, width: usize) -> (String, Option<bool>) {
    const SPIN: [char; 4] = ['|', '/', '-', '\\'];
    let (mark, time, state) = match r.ended {
        Some((end, ok)) => (if ok { '+' } else { 'x' }, took(end - r.started), Some(ok)),
        None => (SPIN[frame % SPIN.len()], took(now - r.started), None),
    };
    let mut line = format!(" {mark} {:<44} {:>10}", r.name, time);
    if state.is_none()
        && let Some(log) = &r.log
    {
        let last = tail(log, 1);
        if !last.is_empty() {
            line.push_str("  ");
            line.push_str(last.trim());
        }
    }
    (line.chars().take(width.saturating_sub(1)).collect(), state)
}

/// Run `f` with the table on the terminal (if standard error is one),
/// then print what each step took and what was said; returns what `f` did.
pub fn show<T: Send>(title: &str, f: impl FnOnce() -> T + Send) -> T {
    use crate::term::{ALT_OFF, ALT_ON, CLEAR, HIDE, SHOW, goto, sgr};
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() || ACTIVE.swap(true, Ordering::SeqCst) {
        return f();
    }
    crate::term::vt_output();
    rows().clear();
    NOTES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let started = Instant::now();
    let mut err = std::io::stderr();
    let _ = write!(err, "{ALT_ON}{HIDE}");
    let result = std::thread::scope(|s| {
        let work = s.spawn(f);
        let mut frame = 0;
        while !work.is_finished() {
            let (width, height) = crate::term::size();
            let now = Instant::now();
            let snapshot = rows().clone();
            let head = format!(" {title}  ({}; the steps run at the same time)", took(now - started));
            let mut screen =
                format!("{CLEAR}{}{}{}", sgr("1;36"), head.chars().take(width).collect::<String>(), sgr("0"));
            for (i, r) in snapshot.iter().rev().take(height.saturating_sub(4)).rev().enumerate() {
                let (text, state) = row_text(r, now, frame, width);
                let colour = match state {
                    Some(true) => "32",
                    Some(false) => "31",
                    None => "33",
                };
                screen.push_str(&format!("{}{}{text}{}", goto(i + 2, 0), sgr(colour), sgr("0")));
            }
            let _ = err.write_all(screen.as_bytes());
            let _ = err.flush();
            frame += 1;
            std::thread::sleep(Duration::from_millis(120));
        }
        work.join()
    });
    let _ = write!(err, "{SHOW}{ALT_OFF}");
    let _ = err.flush();
    ACTIVE.store(false, Ordering::SeqCst);
    let now = Instant::now();
    eprintln!("==> {title}: {}", took(now - started));
    for r in rows().iter() {
        eprintln!("   {}", row_text(r, now, 0, 200).0.trim_start());
    }
    for n in NOTES.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
        eprintln!("{n}");
    }
    match result {
        Ok(r) => r,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_logs_after_their_step() {
        assert_eq!(slug("build native programs (userland/native)"), "build-native-programs-userland-native");
        assert_eq!(slug("build kernel"), "build-kernel");
        assert_eq!(slug("--x--"), "x");
    }

    #[test]
    fn says_how_long() {
        assert_eq!(took(Duration::from_millis(12_340)), "12.3 s");
        assert_eq!(took(Duration::from_secs(125)), "2 min 05 s");
    }

    #[test]
    fn a_row_shows_its_state() {
        let t = Instant::now();
        let r =
            Row { name: "build kernel".into(), started: t, ended: Some((t + Duration::from_secs(3), true)), log: None };
        let (text, state) = row_text(&r, t, 0, 80);
        assert!(text.starts_with(" + build kernel"), "{text}");
        assert!(text.ends_with("3.0 s"), "{text}");
        assert_eq!(state, Some(true));
        let r = Row { ended: None, ..r };
        assert_eq!(row_text(&r, t, 1, 10).0.chars().count(), 9);
        assert_eq!(row_text(&r, t, 1, 80).1, None);
    }
}
