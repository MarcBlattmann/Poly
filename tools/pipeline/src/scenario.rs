//! Scenario drivers (ADR 0088): Windows test programs that run an
//! unchanged application step by step and record what each step did, in
//! an observation file of format `poly-scenario/1`. The first is
//! `tests/windows/sevenzip_scenario.rs`, which runs the pinned `7za.exe`
//! (ADR 0087).
//!
//! The driver runs on the oracle machine through `poly-oracle.ps1` (its
//! data files, `7za.exe`, beside it) and on Poly here ([`run_poly`]); both
//! give a `poly-winrun/1` result whose observation file this module reads
//! ([`parse`]) and compares per step ([`compare`]): the exit code exactly,
//! standard output and error byte for byte after the documented
//! normalizations ([`Normalization`]), and every file by kind, size,
//! SHA-256, last-write time and attributes.
//!
//! The observation file, one record per line:
//!
//! ```text
//! poly-scenario/1 sevenzip steps=15
//! machine <key> <value>                      facts the output may depend on
//! input size=<n> sha256=<hex> name=7za.exe   the program the steps run
//! file size=<n> sha256=<hex> mtime=<t> attr=0x20 name=hello.txt
//! file-b64 hello.txt <base64>                (files up to 64 KiB)
//! step add                                   a step: its records until `end add`
//! command 7za a -bd -sccUTF-8 -mmt1 new.7z hello.txt
//! exit 0x0                                   or spawn-error / timeout / ...
//! stdout size=<n> sha256=<hex>
//! stdout-b64 <base64>
//! stderr ... / stderr-b64 ...
//! file ... / file-b64 ... / dir mtime=<t> attr=0x10 name=out
//! end add
//! done steps=15
//! ```
//!
//! `mtime` is a `FILETIME` value, or `step:<name>` for a time at or after
//! the start of that step (the driver's own clock-free label).

use crate::cpio;
use crate::poly;
use crate::sha256;
use crate::util::{self, Result};
use crate::windows::{self, Manifest, RunFile, RunResult, System, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The observation file's first word.
pub const FORMAT: &str = "poly-scenario/1";
/// The comparison's summary file.
pub const DIFF_FORMAT: &str = "poly-scenario-diff/1";
/// Where data files wait, one directory per test (`<dir>/<test>/<file>`),
/// relative to the repository: what `win bundle --data-dir` ships and
/// [`run_poly`] puts beside the driver.
pub const DATA_DIR: &str = "target/pipeline/windows/data";

// ---------------------------------------------------------------------------
// The observation file

/// Bytes a step wrote to a standard handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captured {
    pub size: u64,
    pub sha256: String,
    /// The bytes, when the driver recorded them.
    pub bytes: Option<Vec<u8>>,
    /// The driver could not read the file (`unreadable error=<n>`).
    pub error: Option<String>,
}

/// A file or directory below the scenario's directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileRec {
    /// Relative path, `/` between the parts.
    pub name: String,
    pub dir: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub mtime: String,
    pub attr: String,
    pub bytes: Option<Vec<u8>>,
    /// `read=` / `unreadable=` notes of the driver.
    pub note: Option<String>,
}

/// One step's records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepRec {
    pub name: String,
    pub command: String,
    /// What follows `exit `: `0x0`, `spawn-error error=2`, `timeout ms=...`.
    pub exit: Option<String>,
    pub stdout: Option<Captured>,
    pub stderr: Option<Captured>,
    pub files: Vec<FileRec>,
    /// The step's `end` record was seen.
    pub ended: bool,
    pub truncated: bool,
}

/// A parsed observation file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scenario {
    /// The scenario's name (`sevenzip`).
    pub name: String,
    pub steps_declared: usize,
    /// `machine <key> <value>`, in order.
    pub machine: Vec<(String, String)>,
    /// `input` records: name to (size, SHA-256).
    pub inputs: BTreeMap<String, (u64, String)>,
    /// The tree before the first step.
    pub initial: Vec<FileRec>,
    pub steps: Vec<StepRec>,
    /// Lines that are not records of the above (`ok ...`, `error ...`,
    /// `summary ...`, a failed `SetFileTime`).
    pub notes: Vec<String>,
    /// The `done` record was seen.
    pub done: bool,
}

impl Scenario {
    pub fn machine(&self, key: &str) -> Option<&str> {
        self.machine.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The local time offset in minutes the machine applied to the pinned
    /// time (`machine local-offset-minutes`), if it reported one.
    pub fn local_offset(&self) -> Option<i64> {
        self.machine("local-offset-minutes")?.parse().ok()
    }

    pub fn step(&self, name: &str) -> Option<&StepRec> {
        self.steps.iter().find(|s| s.name == name)
    }
}

/// Whether `bytes` is a scenario observation file.
pub fn is_scenario(bytes: &[u8]) -> bool {
    bytes.starts_with(FORMAT.as_bytes())
}

/// `key=value` fields before `name=`, and the name (the rest of the line).
fn fields(rest: &str) -> (BTreeMap<&str, &str>, Option<&str>) {
    let (head, name) = match rest.find("name=") {
        Some(0) => ("", Some(&rest[5..])),
        Some(i) if rest.as_bytes()[i - 1] == b' ' => (&rest[..i], Some(&rest[i + 5..])),
        _ => (rest, None),
    };
    (head.split_whitespace().filter_map(|f| f.split_once('=')).collect(), name)
}

fn captured(rest: &str) -> Captured {
    let (f, _) = fields(rest);
    if let Some(e) = f.get("error").filter(|_| rest.starts_with("unreadable")) {
        return Captured { error: Some(format!("unreadable (error {e})")), ..Captured::default() };
    }
    Captured {
        size: f.get("size").and_then(|s| s.parse().ok()).unwrap_or(0),
        sha256: f.get("sha256").unwrap_or(&"").to_string(),
        bytes: None,
        error: None,
    }
}

fn file_rec(rest: &str, dir: bool) -> FileRec {
    let (f, name) = fields(rest);
    let note = ["read", "unreadable"]
        .iter()
        .filter_map(|k| f.get(k).map(|v| format!("{k}={v}")))
        .collect::<Vec<_>>()
        .join(" ");
    FileRec {
        name: name.unwrap_or("").to_string(),
        dir,
        size: f.get("size").and_then(|s| s.parse().ok()),
        sha256: f.get("sha256").map(|s| s.to_string()),
        mtime: f.get("mtime").unwrap_or(&"").to_string(),
        attr: f.get("attr").unwrap_or(&"").to_string(),
        bytes: None,
        note: (!note.is_empty()).then_some(note),
    }
}

/// Read an observation file.
pub fn parse(bytes: &[u8]) -> Result<Scenario> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    let header = lines.next().unwrap_or("");
    let mut words = header.split_whitespace();
    if words.next() != Some(FORMAT) {
        return Err(format!("not a {FORMAT} observation file"));
    }
    let mut s = Scenario { name: words.next().unwrap_or("").to_string(), ..Scenario::default() };
    s.steps_declared = words.find_map(|w| w.strip_prefix("steps=")?.parse().ok()).unwrap_or(0);
    let mut current: Option<StepRec> = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "machine" => {
                let (k, v) = rest.split_once(' ').unwrap_or((rest, ""));
                s.machine.push((k.to_string(), v.to_string()));
            }
            "input" if rest.contains("sha256=") => {
                let (f, name) = fields(rest);
                let size = f.get("size").and_then(|v| v.parse().ok()).unwrap_or(0);
                s.inputs.insert(name.unwrap_or("").to_string(), (size, f.get("sha256").unwrap_or(&"").to_string()));
            }
            "step" => {
                if let Some(st) = current.take() {
                    s.steps.push(st);
                }
                current = Some(StepRec { name: rest.to_string(), ..StepRec::default() });
            }
            "command" if current.is_some() => current.as_mut().unwrap().command = rest.to_string(),
            "exit" if current.is_some() => current.as_mut().unwrap().exit = Some(rest.to_string()),
            "stdout" | "stderr" if current.is_some() => {
                let st = current.as_mut().unwrap();
                let c = Some(captured(rest));
                if word == "stdout" { st.stdout = c } else { st.stderr = c }
            }
            "stdout-b64" | "stderr-b64" if current.is_some() => {
                let st = current.as_mut().unwrap();
                let bytes = windows::unbase64(rest).map_err(|e| format!("{word}: {e}"))?;
                let slot = if word == "stdout-b64" { &mut st.stdout } else { &mut st.stderr };
                slot.get_or_insert_with(Captured::default).bytes = Some(bytes);
            }
            "file" | "dir" => {
                let rec = file_rec(rest, word == "dir");
                match current.as_mut() {
                    Some(st) => st.files.push(rec),
                    None => s.initial.push(rec),
                }
            }
            "file-b64" => {
                let Some((path, b64)) = rest.rsplit_once(' ') else { continue };
                let bytes = windows::unbase64(b64).map_err(|e| format!("file-b64 {path}: {e}"))?;
                let list = match current.as_mut() {
                    Some(st) => &mut st.files,
                    None => &mut s.initial,
                };
                if let Some(f) = list.iter_mut().rev().find(|f| f.name == path) {
                    f.bytes = Some(bytes);
                }
            }
            "tree" if rest == "truncated" => {
                if let Some(st) = current.as_mut() {
                    st.truncated = true;
                }
            }
            "end" => {
                if let Some(mut st) = current.take() {
                    st.ended = st.name == rest;
                    s.steps.push(st);
                }
            }
            "done" => s.done = true,
            _ => s.notes.push(line.to_string()),
        }
    }
    if let Some(st) = current.take() {
        s.steps.push(st);
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Normalizations

/// A documented normalization the comparison applies before calling two
/// outputs different (ADR 0088). Only values that legitimately differ
/// between two correct systems are normalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Normalization {
    /// The reference's `YYYY-MM-DD HH:MM:SS` times in standard output and
    /// error shifted from its local time offset to the target's (7-Zip
    /// prints local times; the offsets are the drivers'
    /// `local-offset-minutes`).
    LocalTime,
    /// The MS-DOS date and time fields of a ZIP archive's headers, which
    /// hold local time (PKWARE's APPNOTE, 4.4.6), shifted the same way.
    ZipDosTime,
}

impl Normalization {
    pub fn describe(self) -> &'static str {
        match self {
            Normalization::LocalTime => "local times shifted by the difference of the time zone offsets",
            Normalization::ZipDosTime => "ZIP MS-DOS times shifted by the difference of the time zone offsets",
        }
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (the usual
/// era-based conversion: 400-year eras of 146097 days).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// Seconds since 1970 of a broken-down time, and back.
fn to_secs(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s
}

fn from_secs(t: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (y, mo, d) = civil_from_days(t.div_euclid(86_400));
    let r = t.rem_euclid(86_400);
    (y, mo, d, r / 3600, r / 60 % 60, r % 60)
}

/// `YYYY-MM-DD HH:MM:SS` (not inside a longer run of digits) shifted by
/// `minutes`.
pub fn shift_timestamps(text: &[u8], minutes: i64) -> Vec<u8> {
    const PATTERN: &[u8; 19] = b"dddd-dd-dd dd:dd:dd";
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let fits = i + 19 <= text.len()
            && PATTERN.iter().zip(&text[i..i + 19]).all(|(&p, &c)| if p == b'd' { c.is_ascii_digit() } else { p == c })
            && (i == 0 || !text[i - 1].is_ascii_digit())
            && text.get(i + 19).is_none_or(|c| !c.is_ascii_digit());
        if !fits {
            out.push(text[i]);
            i += 1;
            continue;
        }
        let n = |a: usize, b: usize| -> i64 {
            std::str::from_utf8(&text[i + a..i + b]).ok().and_then(|s| s.parse().ok()).unwrap_or(0)
        };
        let t = to_secs(n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19)) + minutes * 60;
        let (y, mo, d, h, mi, s) = from_secs(t);
        out.extend_from_slice(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}").as_bytes());
        i += 19;
    }
    out
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// An MS-DOS time and date (Learn: "DosDateTimeToFileTime": date bits
/// 0-4 day, 5-8 month, 9-15 years since 1980; time bits 0-4 seconds / 2,
/// 5-10 minute, 11-15 hour) shifted by `minutes`.
fn shift_dos(time: u16, date: u16, minutes: i64) -> (u16, u16) {
    let (y, mo, d) = (i64::from(date >> 9) + 1980, i64::from(date >> 5 & 15), i64::from(date & 31));
    let (h, mi, s) = (i64::from(time >> 11), i64::from(time >> 5 & 63), i64::from(time & 31) * 2);
    let (y, mo, d, h, mi, s) = from_secs(to_secs(y, mo, d, h, mi, s) + minutes * 60);
    let date = ((y - 1980).clamp(0, 127) as u16) << 9 | (mo as u16) << 5 | d as u16;
    let time = (h as u16) << 11 | (mi as u16) << 5 | (s / 2) as u16;
    (time, date)
}

/// A ZIP archive with the MS-DOS times of its central directory entries
/// and of the local headers they point at shifted by `minutes` (PKWARE
/// APPNOTE 4.3.7, 4.3.12, 4.3.16); `None` unless it parses as one.
pub fn shift_zip_times(zip: &[u8], minutes: i64) -> Option<Vec<u8>> {
    // The end of central directory record, without an archive comment.
    let eocd = zip.len().checked_sub(22)?;
    if le32(zip, eocd)? != 0x0605_4b50 {
        return None;
    }
    let entries = le16(zip, eocd + 10)?;
    let mut at = le32(zip, eocd + 16)? as usize;
    let mut out = zip.to_vec();
    for _ in 0..entries {
        if le32(zip, at)? != 0x0201_4b50 {
            return None;
        }
        let local = le32(zip, at + 42)? as usize;
        if le32(zip, local)? != 0x0403_4b50 {
            return None;
        }
        for (t, d) in [(at + 12, at + 14), (local + 10, local + 12)] {
            let (time, date) = shift_dos(le16(zip, t)?, le16(zip, d)?, minutes);
            out[t..t + 2].copy_from_slice(&time.to_le_bytes());
            out[d..d + 2].copy_from_slice(&date.to_le_bytes());
        }
        at +=
            46 + usize::from(le16(zip, at + 28)?) + usize::from(le16(zip, at + 30)?) + usize::from(le16(zip, at + 32)?);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The comparison

/// How one thing compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    Same,
    /// The same after the normalizations named in the finding.
    Normalized,
    Differs,
    /// The target has no record of it.
    Missing,
}

impl How {
    fn cell(self) -> &'static str {
        match self {
            How::Same => "same",
            How::Normalized => "same*",
            How::Differs => "DIFFERS",
            How::Missing => "-",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// `exit`, `stdout`, `stderr`, `files`.
    pub what: String,
    pub how: How,
    /// The first difference, or the normalizations applied.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepReport {
    pub name: String,
    pub command: String,
    pub verdict: Verdict,
    pub reference_exit: String,
    pub target_exit: Option<String>,
    pub findings: Vec<Finding>,
}

impl StepReport {
    fn finding(&self, what: &str) -> Option<&Finding> {
        self.findings.iter().find(|f| f.what == what)
    }
}

/// The comparison of one scenario run with its reference (`summary.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub format: String,
    pub test: String,
    pub reference: System,
    pub target: System,
    /// Why there are no step results, when there are none.
    #[serde(default)]
    pub problem: Option<String>,
    /// `(fact, reference, target)`, not compared.
    pub machine: Vec<(String, String, String)>,
    /// The target's driver exit code and status.
    pub target_run: String,
    pub normalizations: Vec<Normalization>,
    pub notes: Vec<String>,
    pub steps: Vec<StepReport>,
    pub totals: BTreeMap<String, usize>,
}

impl ScenarioReport {
    /// Every step passed.
    pub fn passed(&self) -> bool {
        self.problem.is_none() && !self.steps.is_empty() && self.steps.iter().all(|s| s.verdict == Verdict::Pass)
    }
}

/// The time offset difference to apply to the reference (target minus
/// reference), and a note when the target did not report one.
fn offset_shift(reference: &Scenario, target: &Scenario, notes: &mut Vec<String>) -> i64 {
    let Some(r) = reference.local_offset() else { return 0 };
    let t = match target.local_offset() {
        Some(t) => t,
        None => {
            notes.push("the target reported no local time offset; UTC (0) assumed for the normalizations".into());
            0
        }
    };
    t - r
}

fn compare_bytes(
    what: &str,
    r: Option<&Captured>,
    t: Option<&Captured>,
    shift: i64,
    used: &mut BTreeSet<Normalization>,
) -> Finding {
    let finding = |how, details: Vec<String>| Finding { what: what.into(), how, details };
    let (r, t) = match (r, t) {
        (Some(r), Some(t)) => (r, t),
        (None, None) => return finding(How::Same, Vec::new()),
        (Some(_), None) => return finding(How::Missing, vec![format!("the target recorded no {what}")]),
        (None, Some(_)) => return finding(How::Differs, vec![format!("only the target recorded {what}")]),
    };
    if let Some(e) = &t.error {
        return finding(How::Differs, vec![format!("target: {e}")]);
    }
    if r.sha256 == t.sha256 && r.size == t.size {
        return finding(How::Same, Vec::new());
    }
    let (Some(rb), Some(tb)) = (&r.bytes, &t.bytes) else {
        return finding(
            How::Differs,
            vec![format!("reference {} bytes, target {} bytes (contents not recorded)", r.size, t.size)],
        );
    };
    if tb.is_empty() {
        return finding(
            How::Differs,
            vec![format!("the target wrote nothing; the reference wrote {} bytes", rb.len())],
        );
    }
    let shifted = if shift != 0 { shift_timestamps(rb, shift) } else { rb.clone() };
    if &shifted == tb {
        used.insert(Normalization::LocalTime);
        return finding(How::Normalized, vec![Normalization::LocalTime.describe().into()]);
    }
    let d = windows::first_difference(what, &shifted, tb).unwrap_or_default();
    let note = if &shifted != rb { format!(", local times shifted by {shift} min") } else { String::new() };
    finding(How::Differs, vec![format!("{d} (reference {} bytes, target {} bytes{note})", rb.len(), tb.len())])
}

/// The first offset at which two byte strings differ.
fn first_byte_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()))
}

fn compare_file(r: &FileRec, t: &FileRec, shift: i64, used: &mut BTreeSet<Normalization>) -> (How, Vec<String>) {
    let mut d = Vec::new();
    let mut how = How::Same;
    if r.dir != t.dir {
        d.push(format!("{}: a {} on the reference, a {} on the target", r.name, kind(r), kind(t)));
        return (How::Differs, d);
    }
    if r.size != t.size {
        d.push(format!("{}: size {} vs {}", r.name, show_opt(r.size), show_opt(t.size)));
    }
    if r.sha256 != t.sha256 && !r.dir {
        let zip = shift != 0 && r.name.to_ascii_lowercase().ends_with(".zip");
        let normalized = match (&r.bytes, &t.bytes) {
            (Some(rb), Some(tb)) if zip => shift_zip_times(rb, shift).is_some_and(|s| &s == tb),
            _ => false,
        };
        if normalized {
            used.insert(Normalization::ZipDosTime);
            how = How::Normalized;
        } else if let (Some(rb), Some(tb)) = (&r.bytes, &t.bytes) {
            let rb = if zip { shift_zip_times(rb, shift).unwrap_or_else(|| rb.clone()) } else { rb.clone() };
            d.push(format!("{}: contents differ from byte {}", r.name, first_byte_difference(&rb, tb)));
        } else {
            d.push(format!("{}: contents differ (SHA-256)", r.name));
        }
    }
    if r.mtime != t.mtime {
        d.push(format!("{}: last-write time {} vs {}", r.name, show_time(&r.mtime), show_time(&t.mtime)));
    }
    if r.attr != t.attr {
        d.push(format!("{}: attributes {} vs {}", r.name, r.attr, t.attr));
    }
    if r.note != t.note {
        d.push(format!("{}: driver note {:?} vs {:?}", r.name, r.note, t.note));
    }
    if d.is_empty() { (how, Vec::new()) } else { (How::Differs, d) }
}

fn kind(f: &FileRec) -> &'static str {
    if f.dir { "directory" } else { "file" }
}

fn show_opt(v: Option<u64>) -> String {
    v.map_or("?".into(), |v| v.to_string())
}

/// A `FILETIME` value as UTC (`2026-01-01 00:00:00Z`), a step label as is.
pub fn show_time(t: &str) -> String {
    match t.parse::<u64>() {
        Ok(v) => {
            let secs = (v / 10_000_000) as i64 - 11_644_473_600;
            let (y, mo, d, h, mi, s) = from_secs(secs);
            let frac = v % 10_000_000;
            if frac == 0 {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{frac:07}Z")
            }
        }
        Err(_) => t.to_string(),
    }
}

fn compare_trees(r: &[FileRec], t: &[FileRec], shift: i64, used: &mut BTreeSet<Normalization>) -> Finding {
    let names: BTreeSet<&str> = r.iter().chain(t).map(|f| f.name.as_str()).collect();
    let mut details = Vec::new();
    let mut how = How::Same;
    for name in names {
        match (r.iter().find(|f| f.name == name), t.iter().find(|f| f.name == name)) {
            (Some(a), Some(b)) => {
                let (h, d) = compare_file(a, b, shift, used);
                if h == How::Normalized && how == How::Same {
                    how = How::Normalized;
                }
                if h == How::Differs {
                    how = How::Differs;
                    details.extend(d);
                }
            }
            (Some(a), None) => {
                how = How::Differs;
                details.push(format!("{name}: missing on the target ({})", kind(a)));
            }
            (None, Some(b)) => {
                how = How::Differs;
                details.push(format!("{name}: only on the target ({})", kind(b)));
            }
            (None, None) => {}
        }
    }
    if how == How::Normalized {
        details.push(Normalization::ZipDosTime.describe().into());
    }
    Finding { what: "files".into(), how, details }
}

fn verdict_of(findings: &[Finding]) -> Verdict {
    if findings.iter().any(|f| f.how == How::Differs) {
        Verdict::Fail
    } else if findings.iter().any(|f| f.how == How::Missing) {
        Verdict::Missing
    } else {
        Verdict::Pass
    }
}

/// Compare a target's scenario run (`target`, its result) with the
/// reference run of the same test.
pub fn compare(
    reference: &RunResult,
    target: Option<&RunResult>,
    systems: (&System, &System),
) -> Result<ScenarioReport> {
    let mut report = ScenarioReport {
        format: DIFF_FORMAT.into(),
        test: reference.name.clone(),
        reference: systems.0.clone(),
        target: systems.1.clone(),
        problem: None,
        machine: Vec::new(),
        target_run: String::new(),
        normalizations: Vec::new(),
        notes: Vec::new(),
        steps: Vec::new(),
        totals: BTreeMap::new(),
    };
    let decode = |r: &RunResult| -> Result<Vec<u8>> {
        r.output_b64.as_deref().map(windows::unbase64).transpose().map(Option::unwrap_or_default)
    };
    if reference.status != "ran" || reference.stable == Some(false) {
        report.problem = Some(format!(
            "the reference is unusable (status {}, stable {:?}): record it again",
            reference.status, reference.stable
        ));
        return Ok(report);
    }
    let r = parse(&decode(reference)?)?;
    let Some(target) = target else {
        report.problem = Some("no target result for this test".into());
        return Ok(report);
    };
    report.target_run = format!(
        "{}, exit {}{}",
        target.status,
        target.exit_code.map_or("none".into(), |c| format!("{c:#x}")),
        target.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default()
    );
    for n in &target.notes {
        report.notes.push(format!("target kernel: {n}"));
    }
    if let (Some(a), Some(b)) = (&reference.exe_sha256, &target.exe_sha256)
        && a != b
    {
        report.problem = Some(format!(
            "different drivers (reference {}, target {}): build the same source, or record the reference again",
            &a[..12.min(a.len())],
            &b[..12.min(b.len())]
        ));
        return Ok(report);
    }
    let out = decode(target)?;
    let t = if is_scenario(&out) { parse(&out)? } else { Scenario::default() };
    if !is_scenario(&out) {
        report.problem = Some(format!(
            "the target wrote no scenario observations (driver run: {}){}",
            report.target_run,
            if out.is_empty() { String::new() } else { format!("; it wrote {} other bytes", out.len()) }
        ));
    }
    let keys: Vec<&str> = {
        let mut k: Vec<&str> = r.machine.iter().map(|(k, _)| k.as_str()).collect();
        for (key, _) in &t.machine {
            if !k.contains(&key.as_str()) {
                k.push(key);
            }
        }
        k
    };
    for k in keys {
        report.machine.push((
            k.to_string(),
            r.machine(k).unwrap_or("-").to_string(),
            t.machine(k).unwrap_or("-").to_string(),
        ));
    }
    if report.problem.is_some() {
        for st in &r.steps {
            report.steps.push(StepReport {
                name: st.name.clone(),
                command: st.command.clone(),
                verdict: Verdict::Missing,
                reference_exit: st.exit.clone().unwrap_or_default(),
                target_exit: None,
                findings: Vec::new(),
            });
        }
        tally(&mut report);
        return Ok(report);
    }
    for (name, (size, sha)) in &r.inputs {
        match t.inputs.get(name) {
            Some((s, h)) if s == size && h == sha => {}
            other => report.notes.push(format!(
                "input {name}: reference {size} bytes {}, target {}",
                &sha[..12.min(sha.len())],
                other.map_or("none".into(), |(s, h)| format!("{s} bytes {}", &h[..12.min(h.len())]))
            )),
        }
    }
    for n in r.notes.iter().filter(|n| !t.notes.contains(n)) {
        report.notes.push(format!("reference only: {n}"));
    }
    for n in t.notes.iter().filter(|n| !r.notes.contains(n)) {
        report.notes.push(format!("target only: {n}"));
    }
    let shift = offset_shift(&r, &t, &mut report.notes);
    let mut used = BTreeSet::new();
    // The tree before the first step: hello.txt with its pinned times.
    let setup = compare_trees(&r.initial, &t.initial, shift, &mut used);
    report.steps.push(StepReport {
        name: "setup".into(),
        command: "(the driver writes hello.txt and sets its times)".into(),
        verdict: verdict_of(std::slice::from_ref(&setup)),
        reference_exit: "-".into(),
        target_exit: Some("-".into()),
        findings: vec![setup],
    });
    for rs in &r.steps {
        let Some(ts) = t.step(&rs.name) else {
            report.steps.push(StepReport {
                name: rs.name.clone(),
                command: rs.command.clone(),
                verdict: Verdict::Missing,
                reference_exit: rs.exit.clone().unwrap_or_default(),
                target_exit: None,
                findings: vec![Finding {
                    what: "step".into(),
                    how: How::Missing,
                    details: vec!["the target's driver did not get to this step".into()],
                }],
            });
            continue;
        };
        let mut findings = Vec::new();
        let exit = if rs.exit == ts.exit {
            Finding { what: "exit".into(), how: How::Same, details: Vec::new() }
        } else {
            Finding {
                what: "exit".into(),
                how: How::Differs,
                details: vec![format!(
                    "reference {}, target {}",
                    rs.exit.as_deref().unwrap_or("none"),
                    ts.exit.as_deref().unwrap_or("none")
                )],
            }
        };
        findings.push(exit);
        findings.push(compare_bytes("stdout", rs.stdout.as_ref(), ts.stdout.as_ref(), shift, &mut used));
        findings.push(compare_bytes("stderr", rs.stderr.as_ref(), ts.stderr.as_ref(), shift, &mut used));
        findings.push(compare_trees(&rs.files, &ts.files, shift, &mut used));
        if rs.command != ts.command {
            findings.push(Finding {
                what: "command".into(),
                how: How::Differs,
                details: vec![format!("reference {:?}, target {:?}", rs.command, ts.command)],
            });
        }
        if !ts.ended {
            findings.push(Finding {
                what: "end".into(),
                how: How::Differs,
                details: vec!["the target's driver did not finish recording this step".into()],
            });
        }
        report.steps.push(StepReport {
            name: rs.name.clone(),
            command: rs.command.clone(),
            verdict: verdict_of(&findings),
            reference_exit: rs.exit.clone().unwrap_or_default(),
            target_exit: ts.exit.clone(),
            findings,
        });
    }
    report.normalizations = used.into_iter().collect();
    tally(&mut report);
    Ok(report)
}

fn tally(report: &mut ScenarioReport) {
    report.totals.clear();
    for s in &report.steps {
        *report.totals.entry(s.verdict.word().to_ascii_lowercase()).or_insert(0) += 1;
    }
}

fn describe(s: &System) -> String {
    let mut out = s.name.clone();
    for part in [&s.version, &s.build].into_iter().flatten() {
        out.push(' ');
        out.push_str(part);
    }
    out
}

fn cell(s: &StepReport, what: &str) -> &'static str {
    s.finding(what).map_or("-", |f| f.how.cell())
}

/// The report as text: one block per step, the totals.
pub fn report_text(r: &ScenarioReport) -> String {
    let mut out = format!(
        "scenario {}\nreference: {}\ntarget:    {}\ntarget run: {}\n",
        r.test,
        describe(&r.reference),
        describe(&r.target),
        r.target_run
    );
    if let Some(p) = &r.problem {
        out.push_str(&format!("PROBLEM: {p}\n"));
    }
    out.push('\n');
    for (k, a, b) in &r.machine {
        out.push_str(&format!("machine {k:<22} reference {a:<40} target {b}\n"));
    }
    for n in &r.notes {
        out.push_str(&format!("note: {n}\n"));
    }
    out.push('\n');
    for s in &r.steps {
        out.push_str(&format!(
            "{:<12} {:<16} exit {} / {}  stdout {}  stderr {}  files {}\n",
            s.verdict.word(),
            s.name,
            s.reference_exit,
            s.target_exit.as_deref().unwrap_or("-"),
            cell(s, "stdout"),
            cell(s, "stderr"),
            cell(s, "files"),
        ));
        for f in &s.findings {
            if f.how == How::Same {
                continue;
            }
            for d in &f.details {
                out.push_str(&format!("             {}: {d}\n", f.what));
            }
        }
    }
    let totals: Vec<String> = r.totals.iter().map(|(k, v)| format!("{v} {k}")).collect();
    out.push_str(&format!("\n{} steps: {}\n", r.steps.len(), totals.join(", ")));
    if !r.normalizations.is_empty() {
        let n: Vec<&str> = r.normalizations.iter().map(|n| n.describe()).collect();
        out.push_str(&format!("normalized (same*): {}\n", n.join("; ")));
    }
    out
}

/// A Markdown table cell: `|` and line ends escaped.
fn md(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// The report as Markdown (`docs/windows/7zip-recording.md`).
pub fn report_markdown(r: &ScenarioReport, reference_path: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# The {} scenario: Windows against Poly\n\n\
         Generated by `cargo xtask win 7zip-diff --markdown docs/windows/7zip-recording.md` \
         ([ADR 0088](../adr/0088-7-zip-scenario-driver.md)); do not edit. Reference: \
         `{reference_path}` ({}); target: {} ({}).\n\n",
        r.test,
        md(&describe(&r.reference)),
        md(&describe(&r.target)),
        md(&r.target_run)
    ));
    if let Some(p) = &r.problem {
        out.push_str(&format!("**Problem:** {}\n\n", md(p)));
    }
    let totals: Vec<String> = r.totals.iter().map(|(k, v)| format!("{v} {k}")).collect();
    out.push_str(&format!("**{} steps:** {}.\n\n", r.steps.len(), totals.join(", ")));
    out.push_str("| Step | Command | Exit (Windows / Poly) | stdout | stderr | files | Verdict |\n");
    out.push_str("|---|---|---|---|---|---|---|\n");
    for s in &r.steps {
        out.push_str(&format!(
            "| {} | `{}` | {} / {} | {} | {} | {} | {} |\n",
            s.name,
            md(&s.command),
            md(&s.reference_exit),
            md(s.target_exit.as_deref().unwrap_or("-")),
            cell(s, "stdout"),
            cell(s, "stderr"),
            cell(s, "files"),
            s.verdict.word()
        ));
    }
    out.push_str(
        "\n`same*`: the same after a documented normalization (ADR 0088); `-`: not recorded by the target.\n\n",
    );
    out.push_str("## Machines\n\nRecorded by the driver, not compared.\n\n| Fact | Windows | Poly |\n|---|---|---|\n");
    for (k, a, b) in &r.machine {
        out.push_str(&format!("| {k} | {} | {} |\n", md(a), md(b)));
    }
    if !r.notes.is_empty() {
        out.push_str("\n## Notes\n\n");
        for n in &r.notes {
            out.push_str(&format!("- {}\n", md(n)));
        }
    }
    out.push_str("\n## Differences per step\n\n");
    let mut any = false;
    for s in &r.steps {
        let bad: Vec<&Finding> = s.findings.iter().filter(|f| matches!(f.how, How::Differs | How::Missing)).collect();
        if bad.is_empty() {
            continue;
        }
        any = true;
        out.push_str(&format!("### {}\n\n", s.name));
        for f in bad {
            for d in &f.details {
                out.push_str(&format!("- **{}**: {}\n", f.what, md(d)));
            }
        }
        out.push('\n');
    }
    if !any {
        out.push_str("None.\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Poly

/// Kernel arguments: the driver as the first program in a directory of its
/// own, and its observation file reported when it has ended. No
/// `poly.test.capture`: only the observation file is reported as fd 1.
pub fn kernel_args(test: &str) -> String {
    format!("poly.qemu_exit poly.init=/{test}/{test}.exe poly.test.stdout=/{test}/{}", windows::OUTPUT_FILE)
}

/// The initramfs: `/<test>/<test>.exe` and the files of `data` beside it.
pub fn initramfs(test: &str, exe: &[u8], data: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut entries = vec![cpio::Entry::dir("dev"), cpio::Entry::dir_mode("tmp", 0o1777), cpio::Entry::dir(test)];
    entries.push(cpio::Entry::file(&format!("{test}/{test}.exe"), 0o755, exe.to_vec()));
    for (name, bytes) in data {
        let mode = if name.to_ascii_lowercase().ends_with(".exe") { 0o755 } else { 0o644 };
        entries.push(cpio::Entry::file(&format!("{test}/{name}"), mode, bytes.clone()));
    }
    cpio::write(&entries)
}

/// The kernel's lines about a run that say what Poly lacks: unhandled
/// exceptions, unimplemented calls, programs it cannot start (each once,
/// at most 30, cut at the first escape sequence).
pub fn kernel_notes(log: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in log.lines() {
        let line = line.split('\x1b').next().unwrap_or("").trim();
        let lower = line.to_ascii_lowercase();
        let wanted = ["unhandled exception", "unimplemented", "not implemented", "poly-init: unsupported"]
            .iter()
            .any(|w| lower.contains(w));
        if wanted && !line.starts_with("POLY-OUT") && !out.iter().any(|o| o == line) && out.len() < 30 {
            out.push(line.to_string());
        }
    }
    out
}

/// Run each scenario driver of `manifest` on Poly with its data files.
pub fn run_poly(
    target: &poly::Target,
    bin: &Path,
    manifest: &Manifest,
    data_dir: &Path,
    work: &Path,
) -> Result<RunFile> {
    let started = util::now_utc();
    let all_data = windows::data_files(manifest, data_dir)?;
    let mut tests = Vec::new();
    for t in &manifest.tests {
        let r = (|| -> Result<RunResult> {
            let exe = util::read(&bin.join(&t.exe))?;
            let data: Vec<(String, Vec<u8>)> = all_data
                .iter()
                .filter(|(test, _, _)| test == &t.name)
                .map(|(_, n, b)| (n.clone(), b.clone()))
                .collect();
            if data.is_empty() {
                eprintln!("==> {}: no data files in {}", t.name, data_dir.join(&t.name).display());
            }
            let image = initramfs(&t.name, &exe, &data)?;
            let (done, log, _) = target.boot_log(&t.name, &image, work, &kernel_args(&t.name))?;
            let timed_out = done.exit == crate::trace::Exit::Timeout;
            let mut r = windows::parse_poly_serial(&t.name, &log, &sha256::hex(&exe), timed_out)?;
            r.ms = Some(done.duration_ns / 1_000_000);
            r.notes = kernel_notes(&log);
            Ok(r)
        })();
        tests.push(r.unwrap_or_else(|e| RunResult {
            name: t.name.clone(),
            exe_sha256: Some(t.exe_sha256.clone()),
            status: "error".into(),
            exit_code_bits: 32,
            error: Some(e),
            ..RunResult::default()
        }));
    }
    Ok(RunFile {
        format: windows::RUN_FORMAT.into(),
        system: System {
            name: "poly".into(),
            version: manifest.revision.as_ref().map(|r| format!("rev {}", &r[..12.min(r.len())])),
            build: None,
            arch: Some("x86_64".into()),
        },
        bundle: None,
        runner: Some("cargo xtask pipeline win scenario poly".into()),
        started: Some(started),
        tests,
    })
}

/// The scenario tests of a result file: those whose observation file is a
/// scenario's.
pub fn scenario_tests(f: &RunFile) -> Vec<&RunResult> {
    f.tests
        .iter()
        .filter(|t| t.output_b64.as_deref().and_then(|b| windows::unbase64(b).ok()).is_some_and(|b| is_scenario(&b)))
        .collect()
}

/// A readable dump of a scenario run: per step the command, the exit and
/// the outputs as text, and the files.
pub fn show(s: &Scenario) -> String {
    let mut out = format!("scenario {} ({} steps declared)\n", s.name, s.steps_declared);
    for (k, v) in &s.machine {
        out.push_str(&format!("machine {k}: {v}\n"));
    }
    for (name, (size, sha)) in &s.inputs {
        out.push_str(&format!("input {name}: {size} bytes, SHA-256 {sha}\n"));
    }
    for n in &s.notes {
        out.push_str(&format!("note: {n}\n"));
    }
    let files = |out: &mut String, list: &[FileRec]| {
        for f in list {
            if f.dir {
                out.push_str(&format!("  dir  {} mtime {} attr {}\n", f.name, show_time(&f.mtime), f.attr));
            } else {
                out.push_str(&format!(
                    "  file {} {} bytes sha256 {} mtime {} attr {}\n",
                    f.name,
                    show_opt(f.size),
                    f.sha256.as_deref().map_or("?", |h| &h[..12.min(h.len())]),
                    show_time(&f.mtime),
                    f.attr
                ));
            }
        }
    };
    out.push_str("before the first step:\n");
    files(&mut out, &s.initial);
    for st in &s.steps {
        out.push_str(&format!(
            "\n== step {}: {}\nexit {}\n",
            st.name,
            st.command,
            st.exit.as_deref().unwrap_or("none")
        ));
        for (what, c) in [("stdout", &st.stdout), ("stderr", &st.stderr)] {
            match c {
                Some(Captured { bytes: Some(b), .. }) if !b.is_empty() => {
                    out.push_str(&format!("-- {what} ({} bytes)\n{}", b.len(), String::from_utf8_lossy(b)));
                    if !b.ends_with(b"\n") {
                        out.push('\n');
                    }
                }
                Some(c) if c.size == 0 && c.error.is_none() => out.push_str(&format!("-- {what}: empty\n")),
                Some(c) => out.push_str(&format!("-- {what}: {} bytes {:?}\n", c.size, c.error)),
                None => out.push_str(&format!("-- {what}: not recorded\n")),
            }
        }
        out.push_str("-- files\n");
        files(&mut out, &st.files);
        if !st.ended {
            out.push_str("(the step's records end here)\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded Windows reference of the 7-Zip scenario.
    fn reference() -> RunFile {
        let path = crate::repo_root().join("tests/windows/oracle/sevenzip_scenario.windows.json");
        RunFile::load(&path).unwrap()
    }

    fn run(name: &str, observation: &str) -> RunResult {
        RunResult {
            name: name.into(),
            exe_sha256: Some("ab".repeat(32)),
            status: "ran".into(),
            exit_code: Some(0),
            exit_code_bits: 32,
            output_b64: Some(windows::base64(observation.as_bytes())),
            stable: Some(true),
            ..RunResult::default()
        }
    }

    const SAMPLE: &str = "poly-scenario/1 demo steps=2\n\
        machine os-version 10.0.26200\n\
        machine local-offset-minutes 60\n\
        input size=3 sha256=aaaa name=7za.exe\n\
        ok sample_written\n\
        file size=2 sha256=bb mtime=134116992000000000 attr=0x20 name=hello.txt\n\
        file-b64 hello.txt aGk=\n\
        step list\n\
        command 7za l x.7z\n\
        exit 0x0\n\
        stdout size=22 sha256=cc\n\
        stdout-b64 MjAyNi0wMS0wMSAwMTowMDowMCBBCg==\n\
        stderr size=0 sha256=e3\n\
        stderr-b64 \n\
        dir mtime=step:list attr=0x10 name=my dir\n\
        file size=2 sha256=bb mtime=step:list attr=0x20 name=my dir/a b.txt\n\
        file-b64 my dir/a b.txt aGk=\n\
        end list\n\
        step fail\n\
        command 7za q\n\
        exit 0x7\n\
        end fail\n\
        done steps=2\n\
        summary 1 passed, 0 failed\n";

    #[test]
    fn parses_an_observation_file() {
        let s = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!((s.name.as_str(), s.steps_declared, s.done), ("demo", 2, true));
        assert_eq!(s.machine("os-version"), Some("10.0.26200"));
        assert_eq!(s.local_offset(), Some(60));
        assert_eq!(s.inputs["7za.exe"], (3, "aaaa".to_string()));
        assert_eq!(s.initial.len(), 1);
        assert_eq!(s.initial[0].bytes.as_deref(), Some(&b"hi"[..]));
        assert_eq!(s.notes, ["ok sample_written", "summary 1 passed, 0 failed"]);
        let list = s.step("list").unwrap();
        assert!(list.ended);
        assert_eq!(list.exit.as_deref(), Some("0x0"));
        assert_eq!(list.stdout.as_ref().unwrap().bytes.as_deref(), Some(&b"2026-01-01 01:00:00 A\n"[..]));
        assert_eq!(list.stderr.as_ref().unwrap().bytes.as_deref(), Some(&b""[..]));
        assert_eq!(list.files[0].name, "my dir");
        assert!(list.files[0].dir);
        assert_eq!(list.files[1].name, "my dir/a b.txt");
        assert_eq!(list.files[1].bytes.as_deref(), Some(&b"hi"[..]));
        assert_eq!(s.step("fail").unwrap().exit.as_deref(), Some("0x7"));
        assert!(parse(b"ok x\n").is_err());
        assert!(!is_scenario(b"ok x\n"));
    }

    #[test]
    fn dates_and_shifts() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 1, 1), 20_454);
        for d in [-800_000, -1, 0, 59, 20_454, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(shift_timestamps(b"at 2026-01-01 01:00:00 A", -60), b"at 2026-01-01 00:00:00 A");
        assert_eq!(shift_timestamps(b"2026-01-01 00:30:00.0000000", -60), b"2025-12-31 23:30:00.0000000");
        assert_eq!(shift_timestamps(b"2024-02-28 23:00:00", 120), b"2024-02-29 01:00:00");
        // Not a time: digits around it, or no time part.
        assert_eq!(shift_timestamps(b"12026-01-01 00:00:00", 60), b"12026-01-01 00:00:00");
        assert_eq!(shift_timestamps(b": 2026-10-21\r\n", 60), b": 2026-10-21\r\n");
        assert_eq!(show_time("134116992000000000"), "2026-01-01 00:00:00Z");
        assert_eq!(show_time("134116992000000001"), "2026-01-01 00:00:00.0000001Z");
        assert_eq!(show_time("step:add"), "step:add");
        // 2026-01-01 01:00:00 as MS-DOS time and date, an hour back.
        let (time, date) = (1u16 << 11, (46u16 << 9) | (1 << 5) | 1);
        assert_eq!(shift_dos(time, date, -60), (0, date));
        assert_eq!(shift_dos(0, date, -60), (23 << 11, (45 << 9) | (12 << 5) | 31));
    }

    #[test]
    fn the_recorded_reference_is_complete_and_shifts_its_zip() {
        let f = reference();
        assert_eq!(f.system.name, "windows");
        let tests = scenario_tests(&f);
        assert_eq!(tests.len(), 1);
        let t = tests[0];
        assert_eq!((t.name.as_str(), t.status.as_str(), t.stable), ("sevenzip_scenario", "ran", Some(true)));
        let s = parse(&windows::unbase64(t.output_b64.as_deref().unwrap()).unwrap()).unwrap();
        assert!(s.done);
        assert_eq!(s.steps.len(), s.steps_declared);
        assert!(s.steps.iter().all(|st| st.ended && st.exit.is_some()));
        assert_eq!(s.inputs["7za.exe"].1, "7ddf0a3bc451e166679296be630d6f0573eb386159c04184761c43020c484291");
        // The reference against itself: every step passes.
        let report = compare(t, Some(t), (&f.system, &f.system)).unwrap();
        assert!(report.passed(), "{}", report_text(&report));
        assert_eq!(report.steps.len(), s.steps.len() + 1);
        // The ZIP's MS-DOS times shift and shift back.
        let zip = s.step("add-zip").unwrap().files.iter().find(|f| f.name == "new.zip").unwrap();
        let bytes = zip.bytes.clone().unwrap();
        let earlier = shift_zip_times(&bytes, -60).unwrap();
        assert_ne!(earlier, bytes);
        assert_eq!(shift_zip_times(&earlier, 60).unwrap(), bytes);
        assert!(shift_zip_times(b"not a zip at all, long enough to look", 0).is_none());
    }

    #[test]
    fn compares_step_by_step() {
        let reference = run("demo", SAMPLE);
        let sys = System { name: "x".into(), ..System::default() };
        // A target in UTC: the listing's local time and nothing else differs.
        let utc = SAMPLE
            .replace("local-offset-minutes 60", "local-offset-minutes 0")
            .replace("MjAyNi0wMS0wMSAwMTowMDowMCBBCg==", &windows::base64(b"2026-01-01 00:00:00 A\n"))
            .replace("stdout size=22 sha256=cc", "stdout size=22 sha256=dd");
        let r = compare(&reference, Some(&run("demo", &utc)), (&sys, &sys)).unwrap();
        assert!(r.passed(), "{}", report_text(&r));
        assert_eq!(r.normalizations, [Normalization::LocalTime]);
        assert_eq!(r.steps[1].finding("stdout").unwrap().how, How::Normalized);

        // A target whose second step failed to start, whose file has other
        // attributes and that never got to finish.
        let bad = SAMPLE
            .replace("exit 0x7", "exit spawn-error error=2")
            .replace("attr=0x20 name=my dir/a b.txt", "attr=0x80 name=my dir/a b.txt")
            .replace("end fail\ndone steps=2\n", "");
        let r = compare(&reference, Some(&run("demo", &bad)), (&sys, &sys)).unwrap();
        assert!(!r.passed());
        let names: Vec<(&str, Verdict)> = r.steps.iter().map(|s| (s.name.as_str(), s.verdict)).collect();
        assert_eq!(names, [("setup", Verdict::Pass), ("list", Verdict::Fail), ("fail", Verdict::Fail)]);
        let text = report_text(&r);
        assert!(text.contains("files: my dir/a b.txt: attributes 0x20 vs 0x80"), "{text}");
        assert!(text.contains("exit: reference 0x7, target spawn-error error=2"), "{text}");
        assert!(text.contains("did not finish recording"), "{text}");
        let md = report_markdown(&r, "ref.json");
        assert!(md.contains("| list | `7za l x.7z` | 0x0 / 0x0 | same | same | DIFFERS | FAIL |"), "{md}");

        // A target that stopped before the second step; one with no
        // observations at all; one of another driver.
        // A target whose program printed nothing (no redirected handles).
        let silent = SAMPLE.replace("stdout-b64 MjAyNi0wMS0wMSAwMTowMDowMCBBCg==", "stdout-b64 ").replace(
            "stdout size=22 sha256=cc",
            "stdout size=0 sha256=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        let mut silent_run = run("demo", &silent);
        silent_run.notes = vec!["nt: pid 3 unhandled exception".into()];
        let r = compare(&reference, Some(&silent_run), (&sys, &sys)).unwrap();
        let text = report_text(&r);
        assert!(text.contains("stdout: the target wrote nothing; the reference wrote 22 bytes"), "{text}");
        assert!(text.contains("note: target kernel: nt: pid 3 unhandled exception"), "{text}");

        let short = SAMPLE.split("step fail").next().unwrap().to_string();
        let r = compare(&reference, Some(&run("demo", &short)), (&sys, &sys)).unwrap();
        assert_eq!(r.steps[2].verdict, Verdict::Missing);
        let mut none = run("demo", "");
        none.exit_code = Some(0xC000_0139);
        let r = compare(&reference, Some(&none), (&sys, &sys)).unwrap();
        assert!(r.problem.as_deref().unwrap().contains("no scenario observations"), "{:?}", r.problem);
        assert!(r.steps.iter().all(|s| s.verdict == Verdict::Missing));
        let mut other = run("demo", SAMPLE);
        other.exe_sha256 = Some("cd".repeat(32));
        let problem = compare(&reference, Some(&other), (&sys, &sys)).unwrap().problem.unwrap();
        assert!(problem.contains("different drivers"), "{problem}");
    }

    #[test]
    fn poly_boot_layout() {
        let args = kernel_args("sevenzip_scenario");
        assert!(args.len() < poly::CMDLINE_MAX);
        assert!(args.contains("poly.init=/sevenzip_scenario/sevenzip_scenario.exe"));
        assert!(args.ends_with("poly.test.stdout=/sevenzip_scenario/poly-test.out"));
        assert!(!args.contains("poly.test.capture"));
        let data = [("7za.exe".to_string(), b"MZ7".to_vec()), ("a.txt".to_string(), b"x".to_vec())];
        let entries = cpio::read(&initramfs("t", b"MZ", &data).unwrap()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        for want in ["t/t.exe", "t/7za.exe", "t/a.txt"] {
            assert!(names.contains(&want), "{names:?}");
        }
        let log = "fs: x\nnt: pid 13 unhandled exception 0xe06d7363 at 0x1\x1b[25;1H\n\
            nt: pid 13 unhandled exception 0xe06d7363 at 0x1\nPOLY-OUT: fd=1 hex=00\nnt: NtFoo unimplemented\n";
        assert_eq!(kernel_notes(log), ["nt: pid 13 unhandled exception 0xe06d7363 at 0x1", "nt: NtFoo unimplemented"]);
    }
}
