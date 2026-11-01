//! The traced 7-Zip scenario (ADR 0110): ADR 0088's fifteen steps, each
//! run through the Windows API tracer's kit (ADR 0090) by the driver
//! `tests/windows/sevenzip_trace.rs`, recorded on the oracle machine and on
//! Poly, and compared call by call.
//!
//! ```text
//! cargo xtask win 7zip-trace stage                       the kit as the driver's data files, the driver built
//! cargo xtask win 7zip-trace record <windows.json>...    the committed reference from Windows runs
//! cargo xtask win 7zip-trace diff [--kernel <elf>] [--build-kernel] [--target <poly.json>]
//!                                 [--markdown <file>] [--check]
//! ```
//!
//! **The reference** (`tests/windows/oracle/sevenzip_trace.reference.json`,
//! format `poly-tracereference/1`) holds each step's exit code, output
//! digests and its trace in **canonical form** ([`wintrace::canonical`]):
//! what the comparison looks at and nothing else (pointers and handles as
//! `NULL` / `-1` / `ptr`, the work directory as `<work>`, results of
//! [`wintrace`]'s volatile functions as `*`). Distinct calls are listed
//! once (`calls`); each step's threads are sequences of indices into that
//! list, `index*count` for a run. That keeps the scenario's 46 000 calls
//! (4.3 MB of raw trace per run) in about 160 KB, and a later trace can be
//! compared with it exactly as with the raw Windows trace.
//!
//! **Recording** (ADR 0110): build and bundle the driver with its data
//! files, run it on the oracle machine through `win remote` (one run per
//! invocation, at least two invocations), then `record` the result files.
//! `record` refuses runs that differ from each other after the documented
//! normalisations, so every kind of run-to-run noise is either normalised
//! (and documented in [`wintrace`]) or visible.
//!
//! **The comparison** (`diff`, and the `wintrace` boot test): per step the
//! exit code, the output digests, and the trace thread by thread
//! ([`wintrace::diff_traces`]): a call only one side made, or the same call
//! with another result, last error, integer or string argument. A step
//! passes with no divergence. The boot test reports; it fails on a
//! difference only when [`TRACE_REQUIRED`] is set.

use crate::wintrace::{self, Call, Divergence, TraceDiff};
use crate::{Result, root};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The driver (`tests/windows/<name>.rs`).
pub const TEST: &str = "sevenzip_trace";
/// The committed reference, from the oracle machine.
pub const REFERENCE: &str = "tests/windows/oracle/sevenzip_trace.reference.json";
/// Where the driver is built (apart from `win build`'s directory).
const BIN: &str = "target/pipeline/windows/trace-bin";
/// Its data files: `<dir>/<test>/` (`pipeline`'s `scenario::DATA_DIR`).
const DATA: &str = "target/pipeline/windows/data";
/// Where `diff` and the boot test put Poly's run, its traces and reports.
const OUT: &str = "target/wintrace/scenario";
/// Whether a difference fails the `wintrace` boot test. Set it to `true`
/// once `cargo xtask win 7zip-trace diff` passes every step (ADR 0110);
/// that is the milestone "7-Zip command line passes its full recorded
/// trace".
pub const TRACE_REQUIRED: bool = false;
/// The observation file's first word (the driver's).
const OBSERVATION_FORMAT: &str = "poly-tracescenario/1";
/// The reference's format.
const REFERENCE_FORMAT: &str = "poly-tracereference/1";
/// What the work directory becomes in strings.
const WORK: &str = "<work>";
/// Poly's boot for the whole scenario (TCG runs the traced steps slowly).
const POLY_TIMEOUT_SECS: u64 = 5400;

// ----- the observation file -----------------------------------------------------------------

/// One step of a traced run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub name: String,
    pub command: String,
    /// What follows `exit `: `0x0`, `spawn-error error=2`, ...
    pub exit: Option<String>,
    /// `size=<n> sha256=<hex>` of standard output and error.
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// The trace file's text, if the driver found it.
    pub trace: Option<String>,
    /// `trace ...` / `trace missing ...` as the driver wrote it.
    pub trace_note: String,
    pub ended: bool,
}

/// A parsed observation file of the traced driver.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observation {
    pub machine: Vec<(String, String)>,
    /// `input` records: name, size, SHA-256 (the kit's files).
    pub inputs: Vec<(String, u64, String)>,
    /// The work directory as the steps saw it (`\` separators).
    pub work_dir: Option<String>,
    pub steps: Vec<Step>,
    /// Other lines (`ok ...`, `error ...`, `summary ...`).
    pub notes: Vec<String>,
    pub done: bool,
}

/// `key=value` words of a record.
fn field<'a>(rest: &'a str, key: &str) -> Option<&'a str> {
    rest.split_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('='))
}

/// Parse the driver's `poly-test.out`.
pub fn parse(bytes: &[u8]) -> Result<Observation> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let header = lines.next().unwrap_or("");
    if header.split_whitespace().next() != Some(OBSERVATION_FORMAT) {
        return Err(format!("not a {OBSERVATION_FORMAT} observation file"));
    }
    let mut o = Observation::default();
    let mut current: Option<Step> = None;
    let mut trace: Option<String> = None;
    for line in lines {
        if let Some(t) = trace.as_mut() {
            if line == "trace-end" {
                if let Some(st) = current.as_mut() {
                    st.trace = trace.take();
                } else {
                    trace = None;
                }
            } else {
                t.push_str(line);
                t.push('\n');
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "machine" => {
                let (k, v) = rest.split_once(' ').unwrap_or((rest, ""));
                o.machine.push((k.to_string(), v.to_string()));
            }
            "input" if rest.contains("sha256=") => {
                let name = rest.split_once("name=").map_or("", |(_, n)| n).to_string();
                let size = field(rest, "size").and_then(|v| v.parse().ok()).unwrap_or(0);
                o.inputs.push((name, size, field(rest, "sha256").unwrap_or("").to_string()));
            }
            "work-dir" => o.work_dir = rest.split_once("path=").map(|(_, p)| p.replace('/', "\\")),
            "step" => {
                if let Some(st) = current.take() {
                    o.steps.push(st);
                }
                current = Some(Step { name: rest.to_string(), ..Step::default() });
            }
            "command" if current.is_some() => current.as_mut().unwrap().command = rest.to_string(),
            "exit" if current.is_some() => current.as_mut().unwrap().exit = Some(rest.to_string()),
            "stdout" | "stderr" if current.is_some() => {
                let st = current.as_mut().unwrap();
                let v = Some(rest.to_string());
                if word == "stdout" { st.stdout = v } else { st.stderr = v }
            }
            "trace" if current.is_some() => {
                current.as_mut().unwrap().trace_note = line.to_string();
                if rest.starts_with("size=") {
                    trace = Some(String::new());
                }
            }
            "stdout-b64" | "stderr-b64" | "file" | "file-b64" | "dir" | "tree" => {}
            "end" => {
                if let Some(mut st) = current.take() {
                    st.ended = st.name == rest;
                    o.steps.push(st);
                }
            }
            "done" => o.done = true,
            _ => o.notes.push(line.to_string()),
        }
    }
    if let Some(mut st) = current.take() {
        if let Some(t) = trace.take() {
            st.trace = Some(t);
        }
        o.steps.push(st);
    }
    Ok(o)
}

/// The string replacements that make a run's paths comparable: its work
/// directory (exactly as long on every system, ADR 0110) becomes `<work>`.
pub fn path_map(o: &Observation) -> Vec<(String, String)> {
    o.work_dir.iter().map(|d| (d.trim_end_matches('\\').to_string(), WORK.to_string())).collect()
}

/// The local time offset the run's machine applies (minutes east of UTC,
/// `machine local-offset-minutes`; 0 if not reported).
pub fn local_offset(o: &Observation) -> i64 {
    o.machine.iter().find(|(k, _)| k == "local-offset-minutes").and_then(|(_, v)| v.parse().ok()).unwrap_or(0)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Every `YYYY-MM-DD HH:MM:SS` in `s` moved by `-minutes`: local times of
/// a machine `minutes` east of UTC as UTC (ADR 0088's local-time
/// normalisation, applied to each run's own strings).
pub fn local_times_to_utc(s: &str, minutes: i64) -> String {
    if minutes == 0 {
        return s.to_string();
    }
    let b = s.as_bytes();
    let shape = b"dddd-dd-dd dd:dd:dd";
    let fits = |at: usize| {
        at + shape.len() <= b.len()
            && shape.iter().zip(&b[at..]).all(|(&p, &c)| if p == b'd' { c.is_ascii_digit() } else { p == c })
    };
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if fits(i) {
            let n = |a: usize, l: usize| s[i + a..i + a + l].parse::<i64>().unwrap_or(0);
            let secs = days_from_civil(n(0, 4), n(5, 2), n(8, 2)) * 86_400 + n(11, 2) * 3600 + n(14, 2) * 60 + n(17, 2)
                - minutes * 60;
            let (y, mo, d) = civil_from_days(secs.div_euclid(86_400));
            let t = secs.rem_euclid(86_400);
            let _ = write!(out, "{y:04}-{mo:02}-{d:02} {:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60);
            i += shape.len();
        } else {
            let c = s[i..].chars().next().unwrap_or('\u{fffd}');
            out.push(c);
            i += c.len_utf8().max(1);
        }
    }
    out
}

/// A step's trace in canonical form (empty without a trace): the work
/// directory as `<work>` and local times in strings as UTC.
pub fn canonical_step(o: &Observation, step: &Step) -> Result<Vec<Call>> {
    let Some(text) = &step.trace else { return Ok(Vec::new()) };
    let calls = wintrace::parse_trace(text).map_err(|e| format!("step {}: {e}", step.name))?;
    let mut calls = wintrace::canonical(&calls, &path_map(o));
    let offset = local_offset(o);
    let utc = |v: &mut Value| {
        if let Value::Object(m) = v {
            for s in m.values_mut() {
                if let Value::String(text) = s {
                    *text = local_times_to_utc(text, offset);
                }
            }
        }
    };
    for c in &mut calls {
        c.args.iter_mut().for_each(&utc);
        utc(&mut c.ret);
    }
    Ok(calls)
}

// ----- result files --------------------------------------------------------------------------

/// RFC 4648 base64 (the result files' `output_b64`).
fn unbase64(s: &str) -> Result<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        acc = acc << 6 | val(c).ok_or_else(|| format!("bad base64 character {:?}", c as char))?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// The driver's run in a `poly-winrun/1` result file: the system's
/// description and the observation.
pub fn load_run(path: &Path) -> Result<(Value, Observation)> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let test = file
        .get("tests")
        .and_then(Value::as_array)
        .and_then(|t| t.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(TEST)))
        .ok_or_else(|| format!("{}: no {TEST} run", path.display()))?;
    let status = test.get("status").and_then(Value::as_str).unwrap_or("?");
    let b64 = test.get("output_b64").and_then(Value::as_str).unwrap_or("");
    if b64.is_empty() {
        let error = test.get("error").and_then(Value::as_str).unwrap_or("");
        return Err(format!("{}: {TEST} {status} without an observation file {error}", path.display()));
    }
    let o = parse(&unbase64(b64)?).map_err(|e| format!("{}: {e}", path.display()))?;
    let system = json!({
        "system": file.get("system").cloned().unwrap_or(Value::Null),
        "started": file.get("started").cloned().unwrap_or(Value::Null),
        "exe_sha256": test.get("exe_sha256").cloned().unwrap_or(Value::Null),
        "status": status,
        "exit_code": test.get("exit_code").cloned().unwrap_or(Value::Null),
    });
    Ok((system, o))
}

// ----- the reference -------------------------------------------------------------------------

/// One step of the reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefStep {
    pub name: String,
    pub command: String,
    pub exit: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub trace_note: String,
    /// Canonical calls, per thread in order.
    pub calls: Vec<Call>,
}

/// The committed reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reference {
    /// Where it comes from (systems, runs, kit inputs, machine facts).
    pub recorded: Value,
    pub steps: Vec<RefStep>,
}

/// A canonical call without its position: `[dll, function, args, result, last error]`.
fn call_key(c: &Call) -> String {
    json!([c.dll, c.func, c.args, c.ret, c.err]).to_string()
}

/// Indices with runs folded: `3 4 5*12 6`.
fn encode_sequence(ids: &[usize]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < ids.len() {
        let mut n = 1;
        while i + n < ids.len() && ids[i + n] == ids[i] {
            n += 1;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "{}", ids[i]);
        if n > 1 {
            let _ = write!(out, "*{n}");
        }
        i += n;
    }
    out
}

fn decode_sequence(s: &str) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for w in s.split_whitespace() {
        let (id, n) = w.split_once('*').unwrap_or((w, "1"));
        let id: usize = id.parse().map_err(|_| format!("bad index {w:?}"))?;
        let n: usize = n.parse().map_err(|_| format!("bad count {w:?}"))?;
        out.extend(std::iter::repeat_n(id, n));
    }
    Ok(out)
}

impl Reference {
    /// The reference's text: JSON, one distinct call per line.
    pub fn to_text(&self) -> String {
        let mut keys: Vec<String> = Vec::new();
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        let mut steps = Vec::new();
        for s in &self.steps {
            let mut threads: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
            for c in &s.calls {
                let k = call_key(c);
                let id = *index.entry(k.clone()).or_insert_with(|| {
                    keys.push(k);
                    keys.len() - 1
                });
                threads.entry(c.t).or_default().push(id);
            }
            let threads: serde_json::Map<String, Value> =
                threads.into_iter().map(|(t, ids)| (t.to_string(), Value::from(encode_sequence(&ids)))).collect();
            steps.push(json!({
                "name": s.name,
                "command": s.command,
                "exit": s.exit,
                "stdout": s.stdout,
                "stderr": s.stderr,
                "trace": s.trace_note,
                "calls": s.calls.len(),
                "threads": threads,
            }));
        }
        let mut out = String::new();
        let _ = writeln!(out, "{{\n\"format\": \"{REFERENCE_FORMAT}\",");
        let recorded = serde_json::to_string_pretty(&self.recorded).unwrap_or_default();
        let _ = writeln!(out, "\"recorded\": {recorded},");
        let _ = writeln!(out, "\"steps\": [");
        for (i, s) in steps.iter().enumerate() {
            let _ = writeln!(out, "{}{}", s, if i + 1 < steps.len() { "," } else { "" });
        }
        let _ = writeln!(out, "],\n\"calls\": [");
        for (i, k) in keys.iter().enumerate() {
            let _ = writeln!(out, "{}{}", k, if i + 1 < keys.len() { "," } else { "" });
        }
        out.push_str("]\n}\n");
        out
    }

    pub fn from_text(text: &str) -> Result<Reference> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("format").and_then(Value::as_str) != Some(REFERENCE_FORMAT) {
            return Err(format!("not a {REFERENCE_FORMAT} file"));
        }
        let calls = v.get("calls").and_then(Value::as_array).ok_or("no calls")?;
        let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
        let mut steps = Vec::new();
        for st in v.get("steps").and_then(Value::as_array).ok_or("no steps")? {
            let mut out = Vec::new();
            for (t, seq) in st.get("threads").and_then(Value::as_object).ok_or("a step without threads")? {
                let t: u64 = t.parse().map_err(|_| format!("bad thread {t:?}"))?;
                for (n, id) in decode_sequence(seq.as_str().unwrap_or(""))?.into_iter().enumerate() {
                    let c = calls.get(id).ok_or_else(|| format!("index {id} beyond the calls"))?;
                    let get = |i: usize| c.get(i).cloned().unwrap_or(Value::Null);
                    out.push(Call {
                        seq: n as u64,
                        t,
                        dll: get(0).as_str().unwrap_or("").to_string(),
                        func: get(1).as_str().unwrap_or("").to_string(),
                        args: get(2).as_array().cloned().unwrap_or_default(),
                        ret: get(3),
                        err: get(4).as_u64(),
                    });
                }
            }
            steps.push(RefStep {
                name: s(st, "name").unwrap_or_default(),
                command: s(st, "command").unwrap_or_default(),
                exit: s(st, "exit"),
                stdout: s(st, "stdout"),
                stderr: s(st, "stderr"),
                trace_note: s(st, "trace").unwrap_or_default(),
                calls: out,
            });
        }
        Ok(Reference { recorded: v.get("recorded").cloned().unwrap_or(Value::Null), steps })
    }

    pub fn load(path: &Path) -> Result<Reference> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Reference::from_text(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The reference from one run (its canonical traces) and where it came from.
pub fn reference_of(o: &Observation, recorded: Value) -> Result<Reference> {
    let mut steps = Vec::new();
    for st in &o.steps {
        steps.push(RefStep {
            name: st.name.clone(),
            command: st.command.clone(),
            exit: st.exit.clone(),
            stdout: st.stdout.clone(),
            stderr: st.stderr.clone(),
            trace_note: st.trace_note.split(" sha256=").next().unwrap_or("").to_string(),
            calls: canonical_step(o, st)?,
        });
    }
    Ok(Reference { recorded, steps })
}

// ----- the comparison ------------------------------------------------------------------------

/// One step compared.
#[derive(Clone, Debug, Default)]
pub struct StepResult {
    pub name: String,
    /// Exit code and output digests: (what, reference, target) where they differ.
    pub outcome: Vec<(String, String, String)>,
    pub calls: (usize, usize),
    pub matched: usize,
    pub diff: TraceDiff,
    /// The target has no trace (or no such step).
    pub missing: Option<String>,
}

impl StepResult {
    pub fn passed(&self) -> bool {
        self.outcome.is_empty() && self.diff.divergences.is_empty() && self.missing.is_none()
    }
}

/// Compare a target's steps with the reference's.
pub fn compare(reference: &Reference, target: &[RefStep]) -> Vec<StepResult> {
    let mut out = Vec::new();
    for r in &reference.steps {
        let mut res = StepResult { name: r.name.clone(), ..StepResult::default() };
        let Some(t) = target.iter().find(|t| t.name == r.name) else {
            res.missing = Some("the target's driver did not get to this step".into());
            res.calls = (r.calls.len(), 0);
            out.push(res);
            continue;
        };
        // The exit code; the outputs are the untraced scenario's to
        // compare (ADR 0088, with its local-time normalisation), so their
        // digests are kept in the reference but not compared here.
        if r.exit != t.exit {
            let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "none".into());
            out_push(&mut res.outcome, "exit", show(&r.exit), show(&t.exit));
        }
        if t.calls.is_empty() && !r.calls.is_empty() {
            res.missing = Some(format!("no trace ({})", if t.trace_note.is_empty() { "none" } else { &t.trace_note }));
        }
        res.diff = wintrace::diff_traces(&r.calls, &t.calls, &[], &[]);
        res.calls = (r.calls.len(), t.calls.len());
        res.matched = res.diff.threads.values().map(|&(_, _, same)| same).sum();
        out.push(res);
    }
    out
}

fn out_push(v: &mut Vec<(String, String, String)>, what: &str, a: String, b: String) {
    v.push((what.to_string(), a, b));
}

/// A value for a report: strings as text, pointers as their class.
fn show_value(v: &Value) -> String {
    match v {
        Value::Object(o) => match o.get("w").or_else(|| o.get("a")) {
            Some(Value::String(s)) => format!("{s:?}"),
            _ => v.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Null => "void".into(),
        // A 32-bit value with its top bit set is also shown signed (the
        // tracer prints 32-bit integers unsigned: memcmp's -1).
        Value::Number(n) => match n.as_u64() {
            Some(u) if (0x8000_0000..=0xFFFF_FFFF).contains(&u) => format!("{u} ({})", u as u32 as i32),
            _ => n.to_string(),
        },
        _ => v.to_string(),
    }
}

/// `dll!fn`, with an ordinal's name (`oleaut32.dll!#2 SysAllocString`).
fn name_of(c: &Call) -> String {
    match c.func.strip_prefix('#').and_then(|o| o.parse().ok()).and_then(|o| wintrace::ordinal_name(&c.dll, o)) {
        Some(n) => format!("{}!{} {n}", c.dll, c.func),
        None => format!("{}!{}", c.dll, c.func),
    }
}

/// `dll!fn(args)`.
fn show_call(c: &Call) -> String {
    let args: Vec<String> = c.args.iter().map(show_value).collect();
    format!("{}({})", name_of(c), args.join(", "))
}

/// `= result, last error n` (or `unchanged`).
fn show_outcome(c: &Call) -> String {
    let e = c.err.map_or_else(|| "last error unchanged".to_string(), |e| format!("last error {e}"));
    format!("= {}, {e}", show_value(&c.ret))
}

/// Per function: (calls only on Windows, only on Poly, differing).
pub fn per_function(results: &[StepResult]) -> BTreeMap<String, (usize, usize, usize)> {
    let mut m: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for r in results {
        for d in &r.diff.divergences {
            match d {
                Divergence::Differs { a, .. } => m.entry(name_of(a)).or_default().2 += 1,
                Divergence::Extra { only_a, only_b, .. } => {
                    for c in only_a {
                        m.entry(name_of(c)).or_default().0 += 1;
                    }
                    for c in only_b {
                        m.entry(name_of(c)).or_default().1 += 1;
                    }
                }
            }
        }
    }
    m
}

/// One divergence as lines of text (`limit` calls of each side shown).
fn divergence_lines(d: &Divergence, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    match d {
        Divergence::Differs { thread, a, b, what } => {
            out.push(format!("thread {thread}, call {}: {}: {}", a.seq, show_call(a), what.join("; ")));
            out.push(format!("    Windows: {} {}", show_call(a), show_outcome(a)));
            out.push(format!("    Poly:    {} {}", show_call(b), show_outcome(b)));
        }
        Divergence::Extra { thread, only_a, only_b, before } => {
            let at = only_a.first().or(only_b.first()).map_or(0, |c| c.seq);
            out.push(format!(
                "thread {thread}, call {at}: {} calls only on Windows, {} only on Poly{}",
                only_a.len(),
                only_b.len(),
                before.as_ref().map(|c| format!(" (after {})", show_call(c))).unwrap_or_default()
            ));
            for (side, calls) in [("Windows", only_a), ("Poly", only_b)] {
                for c in calls.iter().take(limit) {
                    out.push(format!("    only {side}: {} {}", show_call(c), show_outcome(c)));
                }
                if calls.len() > limit {
                    out.push(format!("    only {side}: ... {} more", calls.len() - limit));
                }
            }
        }
    }
    out
}

/// The comparison as text (for the console and the run directory).
pub fn report_text(results: &[StepResult], shown: usize) -> String {
    let mut s = String::new();
    let passed = results.iter().filter(|r| r.passed()).count();
    let _ = writeln!(s, "traced 7-Zip scenario: {passed} of {} steps match the Windows trace", results.len());
    let _ = writeln!(s, "{:<16} {:>8} {:>8} {:>8} {:>6}  outcome", "step", "Windows", "Poly", "matched", "div");
    for r in results {
        let mut what: Vec<String> = r.outcome.iter().map(|(w, a, b)| format!("{w} {a} vs {b}")).collect();
        if let Some(m) = &r.missing {
            what.push(m.clone());
        }
        let _ = writeln!(
            s,
            "{:<16} {:>8} {:>8} {:>8} {:>6}  {}",
            r.name,
            r.calls.0,
            r.calls.1,
            r.matched,
            r.diff.divergences.len(),
            if r.passed() { "PASS".to_string() } else { format!("FAIL {}", what.join("; ")) }
        );
    }
    let per = per_function(results);
    if !per.is_empty() {
        let _ = writeln!(s, "\ndivergences per function (only Windows / only Poly / differing):");
        for (f, (a, b, c)) in &per {
            let _ = writeln!(s, "  {f:<44} {a:>5} {b:>5} {c:>5}");
        }
    }
    for r in results.iter().filter(|r| !r.diff.divergences.is_empty()) {
        let _ = writeln!(s, "\n== {}: {} divergences", r.name, r.diff.divergences.len());
        for d in r.diff.divergences.iter().take(shown) {
            for l in divergence_lines(d, 6) {
                let _ = writeln!(s, "  {l}");
            }
        }
        if r.diff.divergences.len() > shown {
            let _ = writeln!(s, "  ... {} more", r.diff.divergences.len() - shown);
        }
    }
    s
}

fn md(s: &str) -> String {
    s.replace('|', "\\|").replace('<', "&lt;").replace('>', "&gt;")
}

/// The comparison as Markdown: the implementers' feedback
/// (`docs/windows/7zip-trace.md`).
pub fn report_markdown(reference: &Reference, results: &[StepResult], poly: &str) -> String {
    let mut s = String::new();
    let passed = results.iter().filter(|r| r.passed()).count();
    let rec = &reference.recorded;
    let _ = writeln!(s, "# 7-Zip's traced scenario: Poly against Windows\n");
    let _ = writeln!(
        s,
        "Generated by `cargo xtask win 7zip-trace diff --markdown docs/windows/7zip-trace.md` \
         ([ADR 0110](../adr/0110-7-zip-traced-scenario-and-what-passing-its-trace-means.md)). \
         Reference: `{REFERENCE}` ({}, {} runs; build {}). Poly: {poly}.\n",
        rec.pointer("/system/name").and_then(Value::as_str).unwrap_or("?"),
        rec.get("runs").and_then(Value::as_u64).unwrap_or(0),
        rec.pointer("/system/build").and_then(Value::as_str).unwrap_or("?"),
    );
    let _ = writeln!(
        s,
        "**{passed} of {} steps match.** Each step's exit code is compared, and its trace call by \
         call, thread by thread: the function, its result, its last error (set, and to what, or \
         left alone), its integer and string arguments; pointers and handles only as NULL, -1 or \
         \"ptr\". A \"call\" number is the call's index in its thread (thread 0 is the main \
         thread). The outputs and files are the untraced scenario's to compare \
         ([7zip-recording.md](7zip-recording.md)).\n",
        results.len()
    );
    let _ = writeln!(
        s,
        "The documented normalisations (ADR 0110), applied to both sides: the work directory, \
         as long on both systems, is `<work>`; local times in strings are UTC; results of time, \
         tick and id functions and of `GetOEMCP` (the machine's code page) are `*`; \
         `GetVersion` is compared as major.minor (the build is the machine's); \
         `IsProcessorFeaturePresent` calls are left out (the processor's); `memset`'s fill value \
         is compared as a byte. Nothing else is: every other result and last error is compared \
         exactly.\n"
    );
    let _ = writeln!(s, "| Step | Windows calls | Poly calls | Matched | Divergences | Exit code |");
    let _ = writeln!(s, "|---|---:|---:|---:|---:|---|");
    for r in results {
        let mut what: Vec<String> = r.outcome.iter().map(|(w, a, b)| format!("{w}: {} vs {}", md(a), md(b))).collect();
        if let Some(m) = &r.missing {
            what.push(md(m));
        }
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} |",
            r.name,
            r.calls.0,
            r.calls.1,
            r.matched,
            r.diff.divergences.len(),
            if what.is_empty() { "same".to_string() } else { what.join("; ") }
        );
    }
    let per = per_function(results);
    if !per.is_empty() {
        let _ = writeln!(s, "\n## Divergences per function\n");
        let _ =
            writeln!(s, "Over all steps: calls only Windows made, only Poly made, and calls both made that differ.\n");
        let _ = writeln!(s, "| Function | Only Windows | Only Poly | Differ |");
        let _ = writeln!(s, "|---|---:|---:|---:|");
        let mut rows: Vec<_> = per.iter().collect();
        rows.sort_by_key(|(f, (a, b, c))| (std::cmp::Reverse(a + b + c), (*f).clone()));
        for (f, (a, b, c)) in rows {
            let _ = writeln!(s, "| `{f}` | {a} | {b} | {c} |");
        }
    }
    let distinct = distinct_divergences(results);
    if !distinct.is_empty() {
        let _ = writeln!(s, "\n## Each divergence\n");
        let _ = writeln!(
            s,
            "Every distinct divergence once, in the order it first occurs, with how often and in which \
             steps (×n: times in that step) and where it first occurs. \"Differs\": both made the \
             call, with another outcome or argument. \"Only Windows\" / \"only Poly\": calls one side \
             made where the other went on with the next call shown (\"after\" names the last call \
             both made). A divergence can follow from an earlier one in the same step (a last error \
             one side left set changes whether a later call \"changes\" it).\n"
        );
        for (n, g) in distinct.iter().enumerate() {
            let steps: Vec<String> =
                g.steps.iter().map(|(st, k)| if *k > 1 { format!("{st} ×{k}") } else { st.clone() }).collect();
            let times = if g.count == 1 { "once".to_string() } else { format!("{} times", g.count) };
            let _ = writeln!(s, "{}. **{}**: {times}; {}; first: {}", n + 1, md(&g.title), steps.join(", "), g.first);
            let _ = writeln!(s, "   ```text");
            for l in &g.lines {
                let _ = writeln!(s, "   {l}");
            }
            let _ = writeln!(s, "   ```");
        }
    }
    s
}

/// A divergence and its repetitions.
#[derive(Clone, Debug, Default)]
pub struct Distinct {
    /// `dll!function: what`.
    pub title: String,
    /// The calls of both sides, without positions.
    pub lines: Vec<String>,
    pub count: usize,
    /// Steps with the number of times in each, in step order.
    pub steps: Vec<(String, usize)>,
    /// `<step>, thread <t>, call <n>`.
    pub first: String,
}

/// The divergences of all steps, each distinct one once (same calls and
/// outcomes on both sides, wherever they are).
pub fn distinct_divergences(results: &[StepResult]) -> Vec<Distinct> {
    let mut out: Vec<Distinct> = Vec::new();
    for r in results {
        for d in &r.diff.divergences {
            let (title, lines, thread, at) = match d {
                Divergence::Differs { thread, a, b, what } => (
                    format!("{}: {}", name_of(a), what.join("; ")),
                    vec![
                        format!("Windows: {} {}", show_call(a), show_outcome(a)),
                        format!("Poly:    {} {}", show_call(b), show_outcome(b)),
                    ],
                    *thread,
                    a.seq,
                ),
                Divergence::Extra { thread, only_a, only_b, before } => {
                    let mut lines = vec![format!(
                        "after:        {}",
                        before.as_ref().map_or("(the start)".to_string(), |c| format!(
                            "{} {}",
                            show_call(c),
                            show_outcome(c)
                        ))
                    )];
                    for (side, calls) in [("only Windows", only_a), ("only Poly:  ", only_b)] {
                        for c in calls {
                            lines.push(format!("{side}: {} {}", show_call(c), show_outcome(c)));
                        }
                    }
                    let names = |calls: &[Call]| calls.iter().map(|c| c.func.clone()).collect::<Vec<_>>().join(", ");
                    let title = match (only_a.is_empty(), only_b.is_empty()) {
                        (false, true) => format!("only Windows calls {}", names(only_a)),
                        (true, false) => format!("only Poly calls {}", names(only_b)),
                        _ => format!("Windows calls {}, Poly calls {}", names(only_a), names(only_b)),
                    };
                    let at = only_a.first().or(only_b.first()).map_or(0, |c| c.seq);
                    (title, lines, *thread, at)
                }
            };
            match out.iter_mut().find(|g| g.title == title && g.lines == lines) {
                Some(g) => {
                    g.count += 1;
                    match g.steps.last_mut() {
                        Some((st, k)) if *st == r.name => *k += 1,
                        _ => g.steps.push((r.name.clone(), 1)),
                    }
                }
                None => out.push(Distinct {
                    title,
                    lines,
                    count: 1,
                    steps: vec![(r.name.clone(), 1)],
                    first: format!("{}, thread {thread}, call {at}", r.name),
                }),
            }
        }
    }
    out
}

/// A run's steps in the reference's form.
pub fn target_steps(o: &Observation) -> Result<Vec<RefStep>> {
    Ok(reference_of(o, Value::Null)?.steps)
}

// ----- the commands --------------------------------------------------------------------------

const USAGE: &str = "usage: cargo xtask win 7zip-trace <stage | record <windows-results.json>... [--allow-noise] | \
                     diff [--kernel <elf>] [--build-kernel] [--target <poly.json>] [--markdown <file>] [--check]>";

/// Commands that read Windows' observations refuse an implementer's
/// environment (CLEAN_ROOM.md; as `pipeline win diff`).
fn oracle_guard(what: &str) -> Result {
    if std::env::var("POLY_ROLE").is_ok_and(|r| r == "implementer") {
        return Err(format!("{what} reads Windows' observations; an implementer does not run it (CLEAN_ROOM.md)"));
    }
    Ok(())
}

/// The kit for the cached `7za.exe` as the driver's data files, and the
/// driver built.
pub fn stage() -> Result {
    let exe = crate::sevenzip::cached().ok_or("7za.exe is not in the cache; run `cargo xtask win fetch-7zip`")?;
    let data = root().join(DATA).join(TEST);
    let kit = wintrace::build_kit(&exe, &data, &[])?;
    eprintln!("==> trace kit for {} staged in {} ({} proxies)", kit.exe, data.display(), kit.proxies.len());
    let built = crate::sevenzip::pipeline(&["win", "build", "--tests", TEST, "--out", BIN])?;
    if !built.success() {
        return Err(format!("building {TEST}.exe failed ({built})"));
    }
    Ok(())
}

/// Run the driver on Poly (`pipeline win scenario poly`); the result file.
fn run_poly(kernel: Option<&str>, dir: &Path) -> Result<PathBuf> {
    let out = dir.join("poly.json");
    let timeout = POLY_TIMEOUT_SECS.to_string();
    let out_s = out.to_string_lossy().into_owned();
    let mut args =
        vec!["win", "scenario", "poly", "--bin-dir", BIN, "--tests", TEST, "--timeout", &timeout, "--out", &out_s];
    if let Some(k) = kernel {
        args.extend(["--kernel", k]);
    }
    let status = crate::sevenzip::pipeline(&args)?;
    if !status.success() {
        return Err(format!("running {TEST} on Poly failed ({status})"));
    }
    Ok(out)
}

/// Compare a Poly result file with the reference: the results, written
/// to `dir` (report, per-step traces) and, if asked, as Markdown.
fn compare_file(poly: &Path, dir: &Path, markdown: Option<&str>) -> Result<Vec<StepResult>> {
    let reference = Reference::load(&root().join(REFERENCE))?;
    let (system, o) = load_run(poly)?;
    for st in &o.steps {
        if let Some(t) = &st.trace {
            let _ = std::fs::write(dir.join(format!("poly-{}.jsonl", st.name)), t);
        }
    }
    let mut results = compare(&reference, &target_steps(&o)?);
    let problems = setup_problems(&reference, &system, &o);
    if !problems.is_empty() {
        results.insert(
            0,
            StepResult { name: "setup".into(), missing: Some(problems.join("; ")), ..StepResult::default() },
        );
    }
    let text = report_text(&results, 10);
    std::fs::write(dir.join("report.txt"), report_text(&results, usize::MAX)).map_err(|e| e.to_string())?;
    print!("{text}");
    if !o.notes.is_empty() {
        println!("\ndriver notes: {}", o.notes.join(" | "));
    }
    if let Some(m) = markdown {
        let poly_desc = format!(
            "{} ({})",
            system.pointer("/system/name").and_then(Value::as_str).unwrap_or("poly"),
            system.pointer("/system/version").and_then(Value::as_str).unwrap_or("?")
        );
        std::fs::write(root().join(m), report_markdown(&reference, &results, &poly_desc))
            .map_err(|e| format!("{m}: {e}"))?;
        eprintln!("==> wrote {m}");
    }
    Ok(results)
}

/// What makes a run not comparable with the reference: another driver,
/// another kit (the program or a DLL), another work directory length, an
/// unfinished run.
pub fn setup_problems(reference: &Reference, system: &Value, o: &Observation) -> Vec<String> {
    let mut out = Vec::new();
    let rec = &reference.recorded;
    let (want, got) =
        (rec.get("driver_sha256").and_then(Value::as_str), system.get("exe_sha256").and_then(Value::as_str));
    if want.is_some() && got.is_some() && want != got {
        out.push(format!("a different driver ({} against the reference's {})", got.unwrap_or(""), want.unwrap_or("")));
    }
    let inputs: Vec<(String, u64, String)> = rec
        .get("inputs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|i| {
                    let s = |k: &str| i.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    (s("name"), i.get("size").and_then(Value::as_u64).unwrap_or(0), s("sha256"))
                })
                .collect()
        })
        .unwrap_or_default();
    if !inputs.is_empty() && inputs != o.inputs {
        let names: Vec<&str> = o
            .inputs
            .iter()
            .filter(|i| !inputs.contains(i))
            .map(|(n, _, _)| n.as_str())
            .chain(inputs.iter().filter(|i| !o.inputs.contains(i)).map(|(n, _, _)| n.as_str()))
            .collect();
        out.push(format!("a different kit ({}): stage and record again", names.join(", ")));
    }
    let len = o.work_dir.as_ref().map(|d| d.trim_end_matches('\\').chars().count() as u64);
    if let (Some(want), Some(got)) = (rec.get("work_dir_length").and_then(Value::as_u64), len)
        && want != got
    {
        out.push(format!("a work directory of {got} characters, not {want}"));
    }
    if !o.done {
        out.push("the driver did not finish".into());
    }
    out
}

/// `record`: the reference from Windows result files (each one run of the
/// driver); the runs must agree after the normalisations.
fn record(files: &[String], allow_noise: bool) -> Result {
    oracle_guard("win 7zip-trace record")?;
    let [first, rest @ ..] = files else { return Err(USAGE.into()) };
    let (system0, o0) = load_run(Path::new(first))?;
    let reference0 = reference_of(&o0, Value::Null)?;
    let mut noise = 0;
    let mut systems = vec![system0.clone()];
    for f in rest {
        let (system, o) = load_run(Path::new(f))?;
        if system.get("exe_sha256") != system0.get("exe_sha256") || o.inputs != o0.inputs {
            return Err(format!("{f} ran another driver or kit than {first}"));
        }
        let other = target_steps(&o)?;
        let results = compare(&reference0, &other);
        let n: usize = results.iter().map(|r| r.diff.divergences.len() + r.outcome.len()).sum();
        println!("== {first} against {f}: {n} differences");
        print!("{}", report_text(&results, 20));
        noise += n;
        systems.push(system);
    }
    if noise > 0 && !allow_noise {
        return Err(format!(
            "the Windows runs differ ({noise} differences): normalise documented noise in wintrace.rs, or \
             record with --allow-noise and say why"
        ));
    }
    if o0.steps.iter().any(|s| s.trace.is_none()) {
        return Err(format!("{first}: a step without a trace"));
    }
    let recorded = json!({
        "system": system0.get("system"),
        "runs": files.len(),
        "started": systems.iter().map(|s| s.get("started").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "driver_sha256": system0.get("exe_sha256"),
        "work_dir_length": o0.work_dir.as_ref().map(|d| d.trim_end_matches('\\').chars().count()),
        "inputs": o0.inputs.iter().map(|(n, size, sha)| json!({"name": n, "size": size, "sha256": sha})).collect::<Vec<_>>(),
        "machine": o0.machine.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "noise_allowed": noise > 0,
    });
    let reference = reference_of(&o0, recorded)?;
    let text = reference.to_text();
    std::fs::write(root().join(REFERENCE), &text).map_err(|e| format!("{REFERENCE}: {e}"))?;
    let calls: usize = reference.steps.iter().map(|s| s.calls.len()).sum();
    eprintln!("==> wrote {REFERENCE}: {} steps, {calls} calls, {} bytes", reference.steps.len(), text.len());
    Ok(())
}

/// `diff`: stage, run on Poly (or read `--target`), compare.
fn diff(args: &[String]) -> Result {
    oracle_guard("win 7zip-trace diff")?;
    let (mut kernel, mut build_kernel, mut target, mut markdown, mut check) = (None, false, None, None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--kernel" => kernel = Some(it.next().ok_or(USAGE)?.clone()),
            "--build-kernel" => build_kernel = true,
            "--target" => target = Some(PathBuf::from(it.next().ok_or(USAGE)?)),
            "--markdown" => markdown = Some(it.next().ok_or(USAGE)?.clone()),
            "--check" => check = true,
            _ => return Err(USAGE.into()),
        }
    }
    let dir = root().join(OUT);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let poly = match target {
        Some(t) => t,
        None => {
            stage()?;
            if build_kernel || kernel.is_none() {
                kernel = Some(crate::build_kernel(false)?.to_string_lossy().into_owned());
            }
            run_poly(kernel.as_deref(), &dir)?
        }
    };
    let results = compare_file(&poly, &dir, markdown.as_deref())?;
    let failed = results.iter().filter(|r| !r.passed()).count();
    if check && failed > 0 {
        return Err(format!("{failed} steps differ from the Windows trace (report above)"));
    }
    Ok(())
}

/// `cargo xtask win 7zip-trace ...`.
pub fn command(args: &[String]) -> Result {
    match args.first().map(String::as_str) {
        Some("stage") => stage(),
        Some("record") => {
            let allow = args.iter().any(|a| a == "--allow-noise");
            let files: Vec<String> = args[1..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
            record(&files, allow)
        }
        Some("diff") => diff(&args[1..]),
        _ => Err(USAGE.into()),
    }
}

/// The boot test `wintrace`: the traced scenario on the boot tests' kernel,
/// compared with the reference. Fails when the kit or the driver cannot be
/// built, and on a difference only when [`TRACE_REQUIRED`].
pub fn boot_test(kernel: &Path) -> Result {
    eprintln!("==> boot test: the traced 7-Zip scenario on the NT personality, against Windows' trace (ADR 0110)");
    if crate::sevenzip::cached().is_none() {
        eprintln!("==> skipped: 7za.exe is not in the cache; `cargo xtask win fetch-7zip` downloads it");
        return Ok(());
    }
    stage()?;
    let dir = root().join(OUT);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let outcome = run_poly(Some(&kernel.to_string_lossy()), &dir).and_then(|poly| compare_file(&poly, &dir, None));
    let required = if TRACE_REQUIRED { "required" } else { "report only" };
    match outcome {
        Ok(results) if results.iter().all(StepResult::passed) => {
            eprintln!("==> traced 7-Zip scenario: every step matches Windows' trace");
            Ok(())
        }
        Ok(results) => {
            let failed = results.iter().filter(|r| !r.passed()).count();
            eprintln!("==> traced 7-Zip scenario: {failed} steps differ from Windows' trace ({required}, ADR 0110)");
            if TRACE_REQUIRED { Err(format!("{failed} steps of the traced 7-Zip scenario differ")) } else { Ok(()) }
        }
        Err(e) if TRACE_REQUIRED => Err(format!("traced 7-Zip scenario: {e}")),
        Err(e) => {
            eprintln!("==> traced 7-Zip scenario: could not run ({e}; {required})");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBS: &str = "poly-tracescenario/1 sevenzip steps=2\n\
        machine processors 6\n\
        work-dir length=20 path=C:/run/wxxxxxxxxxxxxxx\n\
        input size=3 sha256=ab name=7za.exe\n\
        step info\n\
        command 7za i -bd -sccUTF-8\n\
        exit 0x0\n\
        stdout size=10 sha256=cd\n\
        stdout-b64 aGVsbG8=\n\
        stderr size=0 sha256=e3\n\
        trace size=200 sha256=00 lines=3\n\
        {\"format\":\"polytrace/1\"}\n\
        {\"seq\":1,\"t\":0,\"dll\":\"kernel32.dll\",\"fn\":\"CreateFileW\",\"args\":[{\"w\":\"C:\\\\run\\\\wxxxxxxxxxxxxxx\\\\hello.txt\"},2147483648,\"0x7ff1234\"],\"ret\":\"0xffffffffffffffff\",\"err\":2}\n\
        {\"seq\":0,\"t\":0,\"dll\":\"msvcrt.dll\",\"fn\":\"malloc\",\"args\":[202],\"ret\":\"0x5a0\"}\n\
        trace-end\n\
        file size=1 sha256=00 mtime=1 attr=0x20 name=hello.txt\n\
        end info\n\
        step hash\n\
        command 7za h\n\
        exit 0x2\n\
        trace missing error=2\n\
        end hash\n\
        done steps=2\n";

    #[test]
    fn parses_an_observation() {
        let o = parse(OBS.as_bytes()).unwrap();
        assert_eq!(o.work_dir.as_deref(), Some("C:\\run\\wxxxxxxxxxxxxxx"));
        assert_eq!(o.inputs, vec![("7za.exe".to_string(), 3, "ab".to_string())]);
        assert_eq!(o.steps.len(), 2);
        assert!(o.done && o.steps.iter().all(|s| s.ended));
        assert_eq!(o.steps[0].trace.as_ref().unwrap().lines().count(), 3);
        assert_eq!(o.steps[1].trace, None);
        assert_eq!(o.steps[1].trace_note, "trace missing error=2");
        assert_eq!(o.machine, vec![("processors".to_string(), "6".to_string())]);
        assert!(parse(b"poly-scenario/1 sevenzip").is_err());
    }

    #[test]
    fn canonical_steps_map_the_work_directory_and_pointers() {
        let o = parse(OBS.as_bytes()).unwrap();
        let calls = canonical_step(&o, &o.steps[0]).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].func.as_str(), calls[0].seq), ("malloc", 0));
        assert_eq!(calls[0].ret, json!("ptr"));
        assert_eq!(calls[1].args, vec![json!({"w": "<work>\\hello.txt"}), json!(2147483648u64), json!("ptr")]);
        assert_eq!((calls[1].ret.clone(), calls[1].err), (json!("-1"), Some(2)));
    }

    #[test]
    fn the_reference_round_trips_and_compares_equal() {
        let o = parse(OBS.as_bytes()).unwrap();
        let r = reference_of(&o, json!({"runs": 1})).unwrap();
        let text = r.to_text();
        let back = Reference::from_text(&text).unwrap();
        assert_eq!(back, r);
        let results = compare(&back, &target_steps(&o).unwrap());
        assert!(results.iter().all(StepResult::passed), "{}", report_text(&results, 10));
        assert_eq!(results[0].matched, 2);
    }

    #[test]
    fn sequences_fold_runs() {
        assert_eq!(encode_sequence(&[3, 4, 4, 4, 5, 3]), "3 4*3 5 3");
        assert_eq!(decode_sequence("3 4*3 5 3").unwrap(), vec![3, 4, 4, 4, 5, 3]);
        assert_eq!(encode_sequence(&[]), "");
    }

    #[test]
    fn a_different_target_fails_with_the_divergence() {
        let o = parse(OBS.as_bytes()).unwrap();
        let r = reference_of(&o, Value::Null).unwrap();
        let poly = OBS
            .replace("C:/run/wxxxxxxxxxxxxxx", "Z:/sevenzip_trace/wxx")
            .replace("C:\\\\run\\\\wxxxxxxxxxxxxxx", "Z:\\\\sevenzip_trace\\\\wxx");
        // The same paths under another directory of the same length: equal.
        let p = parse(poly.as_bytes()).unwrap();
        assert!(compare(&r, &target_steps(&p).unwrap()).iter().all(StepResult::passed));
        // Another last error and exit code: two findings.
        let bad = parse(poly.replace("\"err\":2", "\"err\":3").replace("exit 0x2", "exit 0x0").as_bytes()).unwrap();
        let results = compare(&r, &target_steps(&bad).unwrap());
        assert!(results[0].diff.divergences.len() == 1 && !results[0].passed());
        assert_eq!(results[1].outcome, vec![("exit".to_string(), "0x2".to_string(), "0x0".to_string())]);
        let per = per_function(&results);
        assert_eq!(per.get("kernel32.dll!CreateFileW"), Some(&(0, 0, 1)));
        let text = report_text(&results, 10);
        assert!(text.contains("last error 2 vs 3"), "{text}");
        let groups = distinct_divergences(&results);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].count, groups[0].first.as_str()), (1, "info, thread 0, call 1"));
        assert_eq!(groups[0].title, "kernel32.dll!CreateFileW: last error 2 vs 3");
        let md = report_markdown(&r, &results, "poly");
        assert!(md.contains("## Each divergence") && md.contains("| info | 2 | 2 | 1 | 1 |"), "{md}");
        assert!(md.contains("Windows: kernel32.dll!CreateFileW(\"<work>\\\\hello.txt\""), "{md}");
    }

    #[test]
    fn the_committed_reference_is_canonical_and_matches_itself() {
        let text = std::fs::read_to_string(root().join(REFERENCE)).unwrap();
        let r = Reference::from_text(&text).unwrap();
        assert_eq!(r.to_text(), text, "the file is what `record` writes");
        let names: Vec<&str> = r.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 15);
        assert_eq!((names[0], names[14]), ("usage", "bad-command"));
        assert!(r.steps.iter().all(|s| !s.calls.is_empty() && s.exit.is_some()));
        assert_eq!(r.recorded.get("work_dir_length").and_then(Value::as_u64), Some(120));
        let results = compare(&r, &r.steps);
        assert!(results.iter().all(StepResult::passed));
    }

    #[test]
    fn another_driver_or_kit_is_a_setup_problem() {
        let o = parse(OBS.as_bytes()).unwrap();
        let recorded = json!({
            "driver_sha256": "d1",
            "work_dir_length": 22,
            "inputs": [{"name": "7za.exe", "size": 3, "sha256": "ab"}],
        });
        let r = reference_of(&o, recorded).unwrap();
        assert!(setup_problems(&r, &json!({"exe_sha256": "d1"}), &o).is_empty());
        let other = parse(OBS.replace("sha256=ab", "sha256=ff").replace("done steps=2\n", "").as_bytes()).unwrap();
        let p = setup_problems(&r, &json!({"exe_sha256": "d2"}), &other);
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p[0].starts_with("a different driver") && p[1].contains("7za.exe") && p[2].contains("did not finish"));
    }

    #[test]
    fn shows_32_bit_values_signed_too() {
        assert_eq!(show_value(&json!(4294967295u64)), "4294967295 (-1)");
        assert_eq!(show_value(&json!(5)), "5");
        assert_eq!(show_value(&json!({"w": "a"})), "\"a\"");
    }

    #[test]
    fn local_times_become_utc() {
        assert_eq!(local_times_to_utc("2026-01-01 01:00:00.0000000", 60), "2026-01-01 00:00:00.0000000");
        assert_eq!(local_times_to_utc("at 2026-01-01 00:30:00 ok", 60), "at 2025-12-31 23:30:00 ok");
        assert_eq!(local_times_to_utc("2026-02-28 23:00:00", -120), "2026-03-01 01:00:00");
        assert_eq!(local_times_to_utc("ü 2026-01-01 01:00", 60), "ü 2026-01-01 01:00");
        assert_eq!(local_times_to_utc("2026-01-01 01:00:00", 0), "2026-01-01 01:00:00");
        let o = parse(OBS.replace("machine processors 6", "machine local-offset-minutes 60").as_bytes()).unwrap();
        assert_eq!(local_offset(&o), 60);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(unbase64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(unbase64("").unwrap(), b"");
        assert!(unbase64("a$").is_err());
    }
}
