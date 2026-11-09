//! Comparing traces: parsing, normalising, aligning and reporting differences.

use super::*;

// ----- comparing traces -------------------------------------------------------------------

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub seq: u64,
    pub t: u64,
    pub dll: String,
    pub func: String,
    pub args: Vec<Value>,
    /// `Value::Null` for `void` and calls that did not return.
    pub ret: Value,
    pub err: Option<u64>,
}

impl Call {
    fn key(&self) -> (String, &str) {
        (self.dll.to_ascii_lowercase(), self.func.as_str())
    }
}

impl std::fmt::Display for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let args: Vec<String> = self.args.iter().map(Value::to_string).collect();
        write!(f, "{}!{}({}) = {}", self.dll, self.func, args.join(", "), self.ret)?;
        if let Some(e) = self.err {
            write!(f, " [last error {e}]")?;
        }
        write!(f, "  (seq {})", self.seq)
    }
}

/// The calls of a `polytrace.jsonl` (header lines skipped; an unfinished
/// last line, as a cut-off file has, is ignored).
pub fn parse_trace(text: &str) -> Result<Vec<Call>> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) if i + 1 == lines.len() => break,
            Err(e) => return Err(format!("line {}: {e}", i + 1)),
        };
        let Some(seq) = v.get("seq").and_then(Value::as_u64) else { continue };
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        out.push(Call {
            seq,
            t: v.get("t").and_then(Value::as_u64).unwrap_or(0),
            dll: s("dll"),
            func: s("fn"),
            args: v.get("args").and_then(Value::as_array).cloned().unwrap_or_default(),
            ret: v.get("ret").cloned().unwrap_or(Value::Null),
            err: v.get("err").and_then(Value::as_u64),
        });
    }
    Ok(out)
}

/// What is compared of a value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Norm {
    Num(u64),
    Null,
    /// A pointer or handle: NULL, all ones (`INVALID_HANDLE_VALUE`, -1),
    /// or something else, whose value is not compared.
    NullPtr,
    AllOnes,
    Ptr,
    Str(String),
    Other(String),
}

fn norm(v: &Value, map: &[(String, String)]) -> Norm {
    match v {
        Value::Null => Norm::Null,
        Value::Number(n) => n.as_u64().map_or(Norm::Other(n.to_string()), Norm::Num),
        Value::String(s) => match s.strip_prefix("0x").map(|h| u64::from_str_radix(h, 16)) {
            Some(Ok(0)) => Norm::NullPtr,
            Some(Ok(u64::MAX)) => Norm::AllOnes,
            Some(Ok(_)) => Norm::Ptr,
            _ => Norm::Other(s.clone()),
        },
        Value::Object(o) => match o.get("w").or_else(|| o.get("a")).and_then(Value::as_str) {
            Some(s) => {
                let mut s = s.to_string();
                for (from, to) in map {
                    s = s.replace(from.as_str(), to.as_str());
                }
                Norm::Str(s)
            }
            None => Norm::Other(v.to_string()),
        },
        _ => Norm::Other(v.to_string()),
    }
}

/// Functions whose results differ from run to run (times, ids).
const VOLATILE: &[&str] = &[
    "GetTickCount",
    "GetTickCount64",
    "GetCurrentProcessId",
    "GetCurrentThreadId",
    "QueryPerformanceCounter",
    "QueryPerformanceFrequency",
    "GetSystemTimeAsFileTime",
];

/// Results that describe the machine, not the system's behaviour: two
/// correct systems on different machines differ (ADR 0110). `GetOEMCP`:
/// the OEM code page of the machine's locale (850 on the oracle, 437 on
/// Poly; ADR 0088 lists it as a machine fact). Not compared, like
/// [`VOLATILE`]'s.
const MACHINE_RESULTS: &[&str] = &["GetOEMCP"];

/// Functions taken out of both traces before they are aligned (ADR 0110):
/// `IsProcessorFeaturePresent` answers for the processor (`PF_XSAVE_ENABLED`
/// is 1 on the oracle, 0 on QEMU's CPU), and how often a program asks
/// follows `CPUID`, which is not a traced call.
const MACHINE_CALLS: &[&str] = &["IsProcessorFeaturePresent"];

/// Arguments of which only the low byte is the argument (ADR 0110):
/// `memset`'s fill value is converted to `unsigned char` (Learn: "memset");
/// the caller may leave anything in the register's other bits.
const BYTE_ARGS: &[(&str, usize)] = &[("memset", 1)];

/// The documented normalisations of the comparison (ADR 0110), applied to
/// both traces before [`diff_traces`]: [`MACHINE_CALLS`] left out,
/// `GetVersion`'s result as its major and minor version only (the build
/// number in its high word is the machine's Windows build), [`BYTE_ARGS`]
/// masked. Results of [`VOLATILE`] and [`MACHINE_RESULTS`] functions are
/// not compared either (in [`differences`] and [`canonical`]).
pub fn normalise(calls: &[Call]) -> Vec<Call> {
    calls
        .iter()
        .filter(|c| !MACHINE_CALLS.contains(&c.func.as_str()))
        .map(|c| {
            let mut c = c.clone();
            if c.func == "GetVersion"
                && let Some(v) = c.ret.as_u64()
            {
                c.ret = Value::from(format!("version {}.{}", v & 0xff, (v >> 8) & 0xff));
            }
            for &(f, i) in BYTE_ARGS {
                if c.func == f
                    && let Some(v) = c.args.get(i).and_then(Value::as_u64)
                {
                    c.args[i] = Value::from(v & 0xff);
                }
            }
            c
        })
        .collect()
}

/// What the comparison sees of a value, as a value: numbers as they are,
/// strings with `map` applied (`{"w":…}` / `{"a":…}`), pointers and
/// handles as `"NULL"`, `"-1"` (all ones) or `"ptr"`. Two values compare
/// equal in [`diff_traces`] exactly when their canonical forms are equal,
/// and canonical forms compare among themselves the same way, so a trace
/// can be kept in this form (the committed reference of ADR 0110).
pub fn canonical_value(v: &Value, map: &[(String, String)]) -> Value {
    match norm(v, map) {
        Norm::Num(n) => Value::from(n),
        Norm::Null => Value::Null,
        Norm::NullPtr => Value::from("NULL"),
        Norm::AllOnes => Value::from("-1"),
        Norm::Ptr => Value::from("ptr"),
        Norm::Str(s) => {
            let key = if v.get("w").is_some() { "w" } else { "a" };
            serde_json::json!({ key: s })
        }
        Norm::Other(_) => v.clone(),
    }
}

/// `calls` in canonical form: [`normalise`]d, per thread in call order,
/// `seq` renumbered from 0 in each thread (the call's index in its
/// thread), every value [`canonical_value`], and the results
/// [`diff_traces`] does not compare ([`VOLATILE`], [`MACHINE_RESULTS`])
/// replaced by `"*"`.
pub fn canonical(calls: &[Call], map: &[(String, String)]) -> Vec<Call> {
    let calls = normalise(calls);
    let mut sorted: Vec<&Call> = calls.iter().collect();
    sorted.sort_by_key(|c| (c.t, c.seq));
    let mut index: BTreeMap<u64, u64> = BTreeMap::new();
    sorted
        .into_iter()
        .map(|c| {
            let n = index.entry(c.t).or_default();
            let seq = *n;
            *n += 1;
            Call {
                seq,
                t: c.t,
                dll: c.dll.to_ascii_lowercase(),
                func: c.func.clone(),
                args: c.args.iter().map(|a| canonical_value(a, map)).collect(),
                ret: if volatile(c) { Value::from("*") } else { canonical_value(&c.ret, map) },
                err: c.err,
            }
        })
        .collect()
}

/// Whether `c`'s result is not compared.
pub(super) fn volatile(c: &Call) -> bool {
    VOLATILE.contains(&c.func.as_str()) || MACHINE_RESULTS.contains(&c.func.as_str())
}

/// How two calls of the same function differ (empty: they do not).
fn differences(a: &Call, b: &Call, map_a: &[(String, String)], map_b: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    if !volatile(a) && norm(&a.ret, map_a) != norm(&b.ret, map_b) {
        out.push(format!("result {} vs {}", a.ret, b.ret));
    }
    if a.err != b.err {
        let e = |v: Option<u64>| v.map_or_else(|| "unchanged".to_string(), |e| e.to_string());
        out.push(format!("last error {} vs {}", e(a.err), e(b.err)));
    }
    for (i, (x, y)) in a.args.iter().zip(&b.args).enumerate() {
        if norm(x, map_a) != norm(y, map_b) {
            out.push(format!("argument {} {} vs {}", i + 1, x, y));
        }
    }
    out
}

/// One place where two traces part.
#[derive(Clone, Debug, PartialEq)]
pub enum Divergence {
    /// Calls only one side made (the other side's next matching call is
    /// further on); `before`: the last call both made.
    Extra { thread: u64, only_a: Vec<Call>, only_b: Vec<Call>, before: Option<Call> },
    /// The same function, called differently or with another outcome.
    Differs { thread: u64, a: Call, b: Call, what: Vec<String> },
}

/// The comparison of two traces.
#[derive(Clone, Debug, Default)]
pub struct TraceDiff {
    /// Calls per DLL (lower case): A's and B's counts.
    pub per_dll: BTreeMap<String, (usize, usize)>,
    /// Per thread: calls in A, in B, and calls matched with no difference.
    pub threads: BTreeMap<u64, (usize, usize, usize)>,
    pub divergences: Vec<Divergence>,
}

/// How far ahead a resynchronisation looks.
const WINDOW: usize = 64;

/// Compare traces `a` and `b`, thread by thread in call order; `map_a` /
/// `map_b` replace text in string arguments first (e.g. each side's
/// working directory by the same word).
pub fn diff_traces(a: &[Call], b: &[Call], map_a: &[(String, String)], map_b: &[(String, String)]) -> TraceDiff {
    let mut d = TraceDiff::default();
    for c in a {
        d.per_dll.entry(c.dll.to_ascii_lowercase()).or_default().0 += 1;
    }
    for c in b {
        d.per_dll.entry(c.dll.to_ascii_lowercase()).or_default().1 += 1;
    }
    let by_thread = |calls: &[Call]| {
        let mut m: BTreeMap<u64, Vec<Call>> = BTreeMap::new();
        for c in calls {
            m.entry(c.t).or_default().push(c.clone());
        }
        for v in m.values_mut() {
            v.sort_by_key(|c| c.seq);
        }
        m
    };
    let (ta, tb) = (by_thread(a), by_thread(b));
    let mut threads: Vec<u64> = ta.keys().chain(tb.keys()).copied().collect();
    threads.sort_unstable();
    threads.dedup();
    let empty = Vec::new();
    for t in threads {
        let (xa, xb) = (ta.get(&t).unwrap_or(&empty), tb.get(&t).unwrap_or(&empty));
        let (mut i, mut j, mut same) = (0, 0, 0);
        let mut before: Option<Call> = None;
        while i < xa.len() && j < xb.len() {
            if xa[i].key() == xb[j].key() {
                let what = differences(&xa[i], &xb[j], map_a, map_b);
                if what.is_empty() {
                    same += 1;
                } else {
                    d.divergences.push(Divergence::Differs { thread: t, a: xa[i].clone(), b: xb[j].clone(), what });
                }
                before = Some(xa[i].clone());
                i += 1;
                j += 1;
                continue;
            }
            // The nearest pair (i + di, j + dj) that matches again.
            let mut found = None;
            'search: for k in 1..=2 * WINDOW {
                for di in 0..=k.min(WINDOW) {
                    let dj = k - di;
                    if dj > WINDOW || i + di >= xa.len() || j + dj >= xb.len() {
                        continue;
                    }
                    if xa[i + di].key() == xb[j + dj].key() {
                        found = Some((di, dj));
                        break 'search;
                    }
                }
            }
            let (di, dj) = found.unwrap_or((xa.len() - i, xb.len() - j));
            d.divergences.push(Divergence::Extra {
                thread: t,
                only_a: xa[i..i + di].to_vec(),
                only_b: xb[j..j + dj].to_vec(),
                before: before.clone(),
            });
            i += di;
            j += dj;
        }
        if i < xa.len() || j < xb.len() {
            d.divergences.push(Divergence::Extra {
                thread: t,
                only_a: xa[i..].to_vec(),
                only_b: xb[j..].to_vec(),
                before: before.clone(),
            });
        }
        d.threads.insert(t, (xa.len(), xb.len(), same));
    }
    d
}

/// A readable report of `d`, at most `limit` divergences in full.
pub fn report(d: &TraceDiff, name_a: &str, name_b: &str, limit: usize) -> String {
    let mut s = String::new();
    let (na, nb): (usize, usize) = d.per_dll.values().fold((0, 0), |acc, &(x, y)| (acc.0 + x, acc.1 + y));
    let _ = writeln!(s, "A: {name_a} ({na} calls)\nB: {name_b} ({nb} calls)\n");
    let _ = writeln!(s, "{:<24} {:>8} {:>8}", "calls per DLL", "A", "B");
    for (dll, (x, y)) in &d.per_dll {
        let _ = writeln!(s, "{dll:<24} {x:>8} {y:>8}");
    }
    let _ = writeln!(s);
    for (t, (x, y, same)) in &d.threads {
        let _ = writeln!(s, "thread {t}: {x} calls in A, {y} in B, {same} matched without a difference");
    }
    let _ = writeln!(s, "\n{} divergences{}", d.divergences.len(), if d.divergences.is_empty() { "" } else { ":" });
    const SHOWN: usize = 6;
    for (n, div) in d.divergences.iter().take(limit).enumerate() {
        match div {
            Divergence::Differs { thread, a, b, what } => {
                let _ = writeln!(s, "\n{}. thread {thread}: {}!{}: {}", n + 1, a.dll, a.func, what.join("; "));
                let _ = writeln!(s, "   A: {a}\n   B: {b}");
            }
            Divergence::Extra { thread, only_a, only_b, before } => {
                let _ = writeln!(
                    s,
                    "\n{}. thread {thread}: {} calls only in A, {} only in B",
                    n + 1,
                    only_a.len(),
                    only_b.len()
                );
                if let Some(c) = before {
                    let _ = writeln!(s, "   after: {c}");
                }
                for (side, calls) in [("A", only_a), ("B", only_b)] {
                    for c in calls.iter().take(SHOWN) {
                        let _ = writeln!(s, "   {side}: {c}");
                    }
                    if calls.len() > SHOWN {
                        let _ = writeln!(s, "   {side}: ... {} more", calls.len() - SHOWN);
                    }
                }
            }
        }
    }
    if d.divergences.len() > limit {
        let _ = writeln!(s, "\n... {} more divergences", d.divergences.len() - limit);
    }
    s
}
