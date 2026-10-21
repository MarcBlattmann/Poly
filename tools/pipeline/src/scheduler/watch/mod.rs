//! `pipeline loop watch`: a live, read-only dashboard of the loop for a
//! plain terminal (over SSH too): the service and the host, every entry's
//! step, what each worker runs right now and the end of the pipeline log
//! (docs/AGENT_PIPELINE.md, "Watching the loop").
//!
//! ```text
//! loop watch [--config FILE] [--interval SECS] [--once] [--json] [--width N] [--lines N]
//! ```
//!
//! [`gather`] reads (never writes) the state directory, the log and
//! `/proc`; [`render`] makes lines of it; [`term`] draws them every
//! `--interval` seconds (2) on the alternate screen until `q` or Ctrl-C.
//! `--once` prints one snapshot as plain text (colours only on a
//! terminal), `--json` one snapshot as JSON.

pub mod gather;
pub mod render;
pub mod term;

#[cfg(test)]
mod tests;

use super::config::Config;
use crate::util::{Args, Result};
use gather::{Limits, Sources};
use std::path::PathBuf;

/// Where `setup.sh` puts the configuration, used when no other is found
/// (the systemd unit names it with `POLY_PIPELINE_CONFIG`).
const SETUP_CONFIG: &str = "/srv/poly/runner/pipeline.toml";
/// The unit `setup.sh` installs.
const SERVICE: &str = "poly-pipeline";

pub const OPTIONS: &[&str] = &["config", "interval", "once", "json", "width", "lines"];

pub fn command(a: &Args) -> Result<u8> {
    let path = a.opt("config").map(PathBuf::from).or_else(Config::default_path).or_else(|| {
        let p = PathBuf::from(SETUP_CONFIG);
        p.is_file().then_some(p)
    });
    let cfg = match &path {
        Some(p) => Config::load(p).map_err(|e| {
            format!("{e}\n(the configuration and the state belong to the runner user: run this as `server`, e.g. sudo -u server)")
        })?,
        None => Config::default(),
    };
    let interval: f64 = match a.opt("interval") {
        Some(v) => {
            v.parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.2).ok_or("--interval: seconds, at least 0.2")?
        }
        None => 2.0,
    };
    let number = |name: &str| -> Result<Option<usize>> {
        a.opt(name).map(|v| v.parse::<usize>().map_err(|_| format!("--{name}: a number"))).transpose()
    };
    let width = number("width")?;
    let log_lines = number("lines")?.unwrap_or(8);

    let disk_path = ["/srv/poly"]
        .iter()
        .map(PathBuf::from)
        .find(|p| cfg.state_dir.starts_with(p) && p.is_dir())
        .unwrap_or_else(|| cfg.state_dir.clone());
    let src = Sources {
        state_dir: cfg.state_dir.clone(),
        log_file: cfg.log_file.clone(),
        proc_dir: "/proc".into(),
        passwd: "/etc/passwd".into(),
        disk_path,
        service: Some(SERVICE.into()),
        host: hostname(),
        config: path,
        clock_ticks: clock_ticks(),
        log_lines,
    };
    let limits = Limits::of(&cfg);

    if a.has("json") {
        let snap = gather::gather(&src, &limits, crate::util::now_secs());
        println!("{}", serde_json::to_string_pretty(&snap).map_err(|e| e.to_string())?);
        return Ok(0);
    }
    let tty = term::is_tty(libc::STDOUT_FILENO);
    if a.has("once") || !tty {
        let snap = gather::gather(&src, &limits, crate::util::now_secs());
        let width = width.or_else(|| term::size().map(|(w, _)| w)).unwrap_or(120).max(40);
        let color = tty && std::env::var_os("NO_COLOR").is_none();
        for line in render::render(&snap, None) {
            println!("{}", render::to_text(&line, width, color));
        }
        return Ok(0);
    }
    live(&src, &limits, interval, width)
}

fn live(src: &Sources, limits: &Limits, interval: f64, width: Option<usize>) -> Result<u8> {
    let color = std::env::var_os("NO_COLOR").is_none();
    let mut screen = term::Screen::enter();
    let step = std::time::Duration::from_secs_f64(interval);
    loop {
        let snap = gather::gather(src, limits, crate::util::now_secs());
        let (cols, rows) = term::size().unwrap_or((80, 24));
        let cols = width.unwrap_or(cols).max(40);
        // One column short of the edge: a character in the last column
        // leaves the cursor waiting to wrap, and the erase after it would
        // take that character away.
        let lines: Vec<String> =
            render::render(&snap, Some(rows)).iter().map(|l| render::to_text(l, cols - 1, color)).collect();
        screen.draw(&lines);
        let next = std::time::Instant::now() + step;
        loop {
            if term::quit_requested() {
                return Ok(0);
            }
            if term::take_resized() {
                break;
            }
            let left = next.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            match screen.key(left.as_millis() as u64 + 1) {
                Some(b'q' | b'Q' | 0x03 | 0x04) => return Ok(0),
                Some(b'r' | b'R' | b' ') => break,
                _ => {}
            }
        }
    }
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

fn clock_ticks() -> u64 {
    // SAFETY: sysconf only reads a system constant.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(hz).ok().filter(|&h| h > 0).unwrap_or(100)
}
