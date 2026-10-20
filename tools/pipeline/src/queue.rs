//! The work queue: which coverage entry to do next.
//!
//! Unclaimed entries (status `missing` or `stub`) of one OS and layer,
//! ordered by weight (share of target-app calls, from the coverage map),
//! then by when the startup traces first use them (a static "hello world"
//! with and without a C library: what every program needs first), then by
//! syscall number and id.

use crate::trace::Trace;
use crate::util::{self, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct CoverageEntry {
    symbol: String,
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item {
    pub id: String,
    pub status: String,
    pub weight: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// Position of the first use in the startup traces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_use: Option<usize>,
}

/// Position of each syscall name's first use across `traces`, in order.
pub fn first_uses(traces: &[Trace]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for c in traces.iter().flat_map(Trace::syscalls) {
        let next = out.len();
        out.entry(c.name.clone()).or_insert(next);
    }
    out
}

fn order(a: &Item, b: &Item) -> Ordering {
    b.weight
        .partial_cmp(&a.weight)
        .unwrap_or(Ordering::Equal)
        .then_with(|| match (a.first_use, b.first_use) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        .then_with(|| a.number.unwrap_or(u64::MAX).cmp(&b.number.unwrap_or(u64::MAX)))
        .then_with(|| a.id.cmp(&b.id))
}

/// Unclaimed entries of `<os>/<layer>`, best first.
pub fn next(repo: &Path, os: &str, layer: &str, traces: &[Trace]) -> Result<Vec<Item>> {
    let uses = first_uses(traces);
    let dir = repo.join("coverage").join(os).join(layer);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut items = Vec::new();
    for f in files {
        let component = f.file_stem().unwrap().to_string_lossy().into_owned();
        let entries: Vec<CoverageEntry> = util::read_json(&f)?;
        for e in entries {
            let status = e.status.unwrap_or_else(|| "missing".into());
            if status != "missing" && status != "stub" {
                continue;
            }
            items.push(Item {
                id: format!("{os}/{layer}/{component}/{}", e.symbol),
                status,
                weight: e.weight,
                number: e.number,
                first_use: uses.get(&e.symbol).copied(),
            });
        }
    }
    items.sort_by(order);
    Ok(items)
}

/// The startup traces `next` uses by default, from `pipeline trace`'s
/// default directory: the freestanding hello world (the first program
/// Poly runs), then the C library one.
pub fn default_traces(repo: &Path) -> Vec<PathBuf> {
    let dir = repo.join(crate::TRACES_DIR);
    ["hello.json", "libc-hello.json"].iter().map(|f| dir.join(f)).filter(|p| p.is_file()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::tests::call;
    use crate::trace::{Arg, Exit};

    fn startup() -> Trace {
        let mut t = Trace::new("libc-hello", "linux", "ptrace");
        t.exit = Exit::Code(0);
        t.events = vec![
            call("brk", vec![Arg::Ptr(0)], Some(0x1000)),
            call("arch_prctl", vec![Arg::Hex(0x1002), Arg::Ptr(0x1000)], Some(0)),
            call("brk", vec![Arg::Ptr(0x2000)], Some(0x2000)),
            call("write", vec![Arg::Int(1), Arg::Str("\"x\"".into()), Arg::Int(1)], Some(1)),
            call("exit_group", vec![Arg::Int(0)], None),
        ];
        t
    }

    #[test]
    fn first_use_order() {
        let uses = first_uses(&[startup()]);
        assert_eq!(uses["brk"], 0);
        assert_eq!(uses["arch_prctl"], 1);
        assert_eq!(uses["exit_group"], 3);
    }

    #[test]
    fn real_coverage_map_orders_startup_calls_first() {
        let items = next(&crate::repo_root(), "linux", "syscall", &[startup()]).unwrap();
        // Claimed or finished entries never appear.
        assert!(items.iter().all(|i| i.status == "missing" || i.status == "stub"));
        // With equal weights (all 0 today) the traced calls come first, in
        // trace order, then the rest by syscall number.
        if items.iter().all(|i| i.weight == 0.0) {
            let traced: Vec<usize> = items.iter().map_while(|i| i.first_use).collect();
            assert!(traced.windows(2).all(|w| w[0] < w[1]), "{traced:?}");
            let rest = &items[traced.len()..];
            assert!(rest.iter().all(|i| i.first_use.is_none()));
            assert!(rest.windows(2).all(|w| w[0].number <= w[1].number));
        }
    }

    #[test]
    fn weight_beats_first_use() {
        let a = Item { id: "a".into(), status: "missing".into(), weight: 0.0, number: Some(1), first_use: Some(0) };
        let b = Item { id: "b".into(), status: "missing".into(), weight: 0.5, number: Some(2), first_use: None };
        let c = Item { id: "c".into(), status: "missing".into(), weight: 0.0, number: Some(0), first_use: None };
        let mut v = vec![c.clone(), a.clone(), b.clone()];
        v.sort_by(order);
        assert_eq!(v, [b, a, c]);
    }
}
