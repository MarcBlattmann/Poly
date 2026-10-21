//! A [`Snapshot`] as lines of text for a plain terminal: ASCII only,
//! every line cut to the width (80 columns and up), colours as ANSI SGR
//! codes when wanted.

use super::gather::{Activity, Row, Snapshot};
use crate::util;

/// A piece of a line in one style (an SGR parameter string, "" = plain).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: &'static str,
}

pub type Line = Vec<Span>;

const BOLD: &str = "1";
const DIM: &str = "2";
const RED: &str = "1;31";
const GREEN: &str = "32";
const YELLOW: &str = "33";
const CYAN: &str = "36";

fn span(text: impl Into<String>, style: &'static str) -> Span {
    Span { text: text.into(), style }
}

fn plain(text: impl Into<String>) -> Span {
    span(text, "")
}

/// The colour of a step.
pub fn stage_style(stage: &str) -> &'static str {
    match stage {
        "claim" => "37",
        "spec" => CYAN,
        "spec-check" => "34",
        "handoff" => "35",
        "implement" => YELLOW,
        "verify" => "94",
        "publish" => "92",
        "waiting" => "96",
        "refresh" => "93",
        "human" => RED,
        "done" => "1;32",
        _ => "",
    }
}

/// `secs` as a short duration: 42s, 12m05s, 3h04m, 2d05h.
pub fn duration(secs: u64) -> String {
    match secs {
        0..100 => format!("{secs}s"),
        100..3600 => format!("{}m{:02}s", secs / 60, secs % 60),
        3600..86_400 => format!("{}h{:02}m", secs / 3600, secs / 60 % 60),
        _ => format!("{}d{:02}h", secs / 86_400, secs / 3600 % 24),
    }
}

/// A time of day (`14:03 UTC`), with the date when it is not today.
pub fn clock(at: u64, now: u64) -> String {
    let t = util::format_utc(at);
    if at / 86_400 == now / 86_400 {
        format!("{} UTC", &t[11..16])
    } else {
        format!("{} {} UTC", &t[5..10], &t[11..16])
    }
}

/// `at` relative to `now`: "in 5m00s", "3m00s ago", "due".
fn relative(at: u64, now: u64) -> String {
    if at > now {
        format!("in {}", duration(at - now))
    } else if now - at < 5 {
        "due".into()
    } else {
        format!("{} ago", duration(now - at))
    }
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 || v >= 100.0 { format!("{v:.0}{}", UNITS[u]) } else { format!("{v:.1}{}", UNITS[u]) }
}

/// The entry id without the shared prefix.
fn short<'a>(entry: &'a str, prefix: &str) -> &'a str {
    entry.strip_prefix(prefix).unwrap_or(entry)
}

/// What every entry on screen starts with, in whole `/` parts and never
/// the symbol (`linux/syscall/x86_64/` when all are such syscalls), else
/// the configured `<os>/<layer>/`: left out of the ids on screen.
pub fn shared_prefix(s: &Snapshot) -> String {
    let ids: Vec<&str> = s
        .entries
        .iter()
        .map(|r| r.entry.as_str())
        .chain(s.activity.iter().filter_map(|a| a.entry.as_deref()))
        .collect();
    let Some(first) = ids.first() else { return s.limits.id_prefix.clone() };
    let parts: Vec<&str> = first.split('/').collect();
    let mut n = parts.len() - 1;
    for id in &ids[1..] {
        let other: Vec<&str> = id.split('/').collect();
        n = n.min(other.len() - 1).min(parts.iter().zip(&other).take_while(|(a, b)| a == b).count());
    }
    let prefix: String = parts[..n].iter().map(|p| format!("{p}/")).collect();
    if prefix.len() >= s.limits.id_prefix.len() || !s.limits.id_prefix.starts_with(&prefix) {
        prefix
    } else {
        s.limits.id_prefix.clone()
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pad(text: &str, width: usize) -> String {
    let mut t: String = text.chars().take(width).collect();
    let n = t.chars().count();
    t.extend(std::iter::repeat_n(' ', width - n));
    t
}

/// The dashboard. `height`: rows available (`None`: everything, for
/// `--once`); the entry table gives way first.
pub fn render(s: &Snapshot, height: Option<usize>) -> Vec<Line> {
    let now = s.time;
    let prefix = shared_prefix(s);
    let mut head: Vec<Line> = Vec::new();
    head.push(vec![
        span("POLY PIPELINE", BOLD),
        plain(format!("  {}  {} UTC", s.host, util::format_utc(now))),
        span(if height.is_some() { "   q quits, r refreshes" } else { "" }, DIM),
    ]);

    let mut l = vec![plain("service ")];
    l.push(match s.service.as_deref() {
        Some("active") => span("active", GREEN),
        Some(other) => span(other.to_string(), RED),
        None => span("unknown", DIM),
    });
    match s.loops.as_slice() {
        [] => l.push(span("  no loop process seen", YELLOW)),
        [p] => l.push(plain(format!("  loop pid {} up {}", p.pid, duration(now.saturating_sub(p.started))))),
        ps => l.push(span(format!("  {} loop processes", ps.len()), YELLOW)),
    }
    l.push(plain(format!("  max_parallel {}", s.limits.max_parallel)));
    head.push(l);
    head.push(vec![plain(format!("config {}  state {}", s.config.as_deref().unwrap_or("(defaults)"), s.state_dir))]);

    let mut l = Vec::new();
    match s.load {
        Some([a, b, c]) => l.push(plain(format!("load {a:.2} {b:.2} {c:.2}"))),
        None => l.push(span("load ?", DIM)),
    }
    if let Some(m) = &s.memory {
        let low = m.free * 10 < m.total;
        l.push(plain("  mem "));
        l.push(span(format!("{}/{} free", bytes(m.free), bytes(m.total)), if low { RED } else { "" }));
    }
    if let Some(d) = &s.disk {
        let low = d.space.free * 10 < d.space.total;
        l.push(plain("  disk "));
        l.push(span(format!("{}/{} free", bytes(d.space.free), bytes(d.space.total)), if low { RED } else { "" }));
        l.push(plain(format!(" on {}", d.path)));
    }
    head.push(l);

    let paused: Vec<_> = s.pauses.iter().filter(|p| p.active).collect();
    if paused.is_empty() {
        let busy = if s.roles_busy.is_empty() { "none".into() } else { s.roles_busy.join(", ") };
        head.push(vec![plain(format!("roles: none paused; in a session: {busy}"))]);
    }
    for p in paused {
        head.push(vec![
            span(format!("PAUSED {}", p.role), YELLOW),
            plain(format!(" until {} ({}): {}", clock(p.until, now), relative(p.until, now), one_line(&p.reason))),
        ]);
    }

    let t = &s.totals;
    head.push(vec![
        span(format!("done today {}", t.done_today), if t.done_today > 0 { GREEN } else { "" }),
        plain(format!(" ({} in all)  ", t.done)),
        span(format!("human queue {}", t.human), if t.human > 0 { RED } else { "" }),
        plain(format!("  open PRs {}  in flight {}/{}", t.open_prs, t.in_flight, s.limits.max_parallel)),
    ]);
    for p in &s.problems {
        head.push(vec![span(format!("! {}", one_line(p)), YELLOW)]);
    }

    let activity = activity_lines(s, &prefix, now);
    let mut log: Vec<Line> = Vec::new();
    log.push(vec![span(
        match &s.log_file {
            Some(f) => format!("LOG {f}"),
            None => "LOG (no log_file configured: journalctl -u poly-pipeline)".into(),
        },
        BOLD,
    )]);
    log.extend(s.log_tail.iter().map(|l| vec![span(log_line(l, &prefix, now), DIM)]));

    let (table_head, rows) = entry_lines(s, &prefix, now);
    let mut out = head;
    out.push(Vec::new());
    out.push(vec![span(
        if prefix.is_empty() { "ENTRIES".to_string() } else { format!("ENTRIES ({prefix}...)") },
        BOLD,
    )]);
    out.push(table_head);
    let fixed = out.len() + 2 + activity.len() + 1 + log.len();
    let room = height.map_or(usize::MAX, |h| h.saturating_sub(fixed));
    if rows.is_empty() {
        out.push(vec![span("  no entries yet", DIM)]);
    } else if rows.len() <= room {
        out.extend(rows);
    } else {
        let shown = room.saturating_sub(1);
        let hidden = rows.len() - shown;
        out.extend(rows.into_iter().take(shown));
        out.push(vec![span(format!("  ... {hidden} more (loop status lists all)"), DIM)]);
    }
    out.push(Vec::new());
    out.push(vec![span("ACTIVITY", BOLD)]);
    out.extend(activity);
    out.push(Vec::new());
    out.extend(log);
    if let Some(h) = height {
        out.truncate(h);
    }
    out
}

fn id_width(s: &Snapshot, prefix: &str) -> usize {
    s.entries.iter().map(|r| short(&r.entry, prefix).len()).max().unwrap_or(5).clamp(5, 24)
}

fn entry_lines(s: &Snapshot, prefix: &str, now: u64) -> (Line, Vec<Line>) {
    let id_w = id_width(s, prefix);
    let head = vec![span(
        format!(
            "  {} {} {} {} {} NOTE",
            pad("ENTRY", id_w),
            pad("STEP", 10),
            pad("CYCLE", 5),
            pad("ERR", 4),
            pad("IN STEP", 7)
        ),
        BOLD,
    )];
    let rows = s
        .entries
        .iter()
        .map(|r| {
            let cycle = match r.stage.as_str() {
                "spec" | "spec-check" if r.spec_cycle > 0 => format!("s{}/{}", r.spec_cycle, s.limits.spec_cycles),
                _ => format!("{}/{}", r.cycle, s.limits.max_cycles),
            };
            let errors = format!("{}/{}", r.errors, s.limits.max_errors);
            let in_step = match (r.stage.as_str(), r.since) {
                ("done", _) | (_, None) => "-".into(),
                (_, Some(t)) => duration(now.saturating_sub(t)),
            };
            vec![
                span(if r.running { "* " } else { "  " }, GREEN),
                plain(format!("{} ", pad(short(&r.entry, prefix), id_w))),
                span(pad(&r.stage, 10), stage_style(&r.stage)),
                span(
                    format!(" {}", pad(&cycle, 5)),
                    if r.cycle + 1 >= s.limits.max_cycles && r.cycle > 0 { RED } else { "" },
                ),
                span(format!(" {}", pad(&errors, 4)), if r.errors > 0 { YELLOW } else { "" }),
                plain(format!(" {} ", pad(&in_step, 7))),
                note(r, now),
            ]
        })
        .collect();
    (head, rows)
}

fn note(r: &Row, now: u64) -> Span {
    if r.stage == "human" {
        return span(one_line(r.reason.as_deref().unwrap_or("see the human queue")), RED);
    }
    let text = if let Some(url) = &r.pr_url {
        let url = url.trim_start_matches("https://");
        match r.stage.as_str() {
            "waiting" => match &r.wait {
                Some(w) => format!("{url}, check {}", relative(w.until, now)),
                None => url.to_string(),
            },
            _ => url.to_string(),
        }
    } else if r.running {
        format!("running{}", r.worker.as_ref().map_or(String::new(), |w| format!(" in {w}")))
    } else if let Some(w) = &r.wait {
        format!("{} {} ({})", w.what, relative(w.until, now), clock(w.until, now))
    } else {
        r.reason.as_deref().map_or("ready".into(), one_line)
    };
    span(text, if r.running { GREEN } else { "" })
}

fn activity_lines(s: &Snapshot, prefix: &str, now: u64) -> Vec<Line> {
    if s.activity.is_empty() {
        let why = if s.loops.is_empty() { "nothing running (no loop process seen)" } else { "workers idle" };
        return vec![vec![span(format!("  {why}"), DIM)]];
    }
    let id_w = id_width(s, prefix);
    s.activity.iter().flat_map(|a| activity_line(a, prefix, id_w, now)).collect()
}

/// One or two lines: the command, then the agent process and the log.
fn activity_line(a: &Activity, prefix: &str, id_w: usize, now: u64) -> Vec<Line> {
    let mut l = vec![plain(format!(
        "  {:<3} {} ",
        a.worker.as_deref().unwrap_or("-"),
        pad(a.entry.as_deref().map_or("(loop)", |e| short(e, prefix)), id_w)
    ))];
    let style = match a.what.as_str() {
        w if w.ends_with("session") => YELLOW,
        "CI" | "diff run" => "94",
        w if w.contains("between commands") => DIM,
        _ => "",
    };
    l.push(span(a.what.clone(), style));
    if let Some(t) = a.started {
        l.push(plain(format!(" {}{}", if a.exact { "" } else { "~" }, duration(now.saturating_sub(t)))));
    }
    if let Some(pid) = a.pid {
        l.push(span(format!("  pid {pid}"), DIM));
    }
    let mut more = Vec::new();
    if let Some(ag) = &a.agent {
        more.push(format!(
            "{} pid {}{} up {}",
            ag.program,
            ag.pid,
            ag.user.as_ref().map_or(String::new(), |u| format!(" as {u}")),
            duration(now.saturating_sub(ag.started))
        ));
    }
    if let Some(age) = a.log_age {
        more.push(format!("log written {} ago", duration(age)));
    }
    if more.is_empty() {
        return vec![l];
    }
    vec![l, vec![span(format!("{}{}", " ".repeat(7 + id_w), more.join("; ")), DIM)]]
}

/// A pipeline log line, shorter: no date when it is today's, entry ids
/// without `<os>/<layer>/`.
fn log_line(line: &str, prefix: &str, now: u64) -> String {
    let today = &util::format_utc(now)[..11];
    let line = line.strip_prefix(today).unwrap_or(line);
    line.replace(&format!(" {prefix}"), " ")
}

/// A line as text: cut to `width` characters, with ANSI colours if `color`.
pub fn to_text(line: &Line, width: usize, color: bool) -> String {
    let mut out = String::new();
    let mut left = width;
    for s in line {
        if left == 0 {
            break;
        }
        let text: String = s.text.chars().filter(|c| !c.is_control()).take(left).collect();
        left -= text.chars().count();
        if color && !s.style.is_empty() {
            out.push_str(&format!("\x1b[{}m{text}\x1b[0m", s.style));
        } else {
            out.push_str(&text);
        }
    }
    if !color {
        return out.trim_end().to_string();
    }
    out
}
