//! `coverage/coverage.json`: the single source of truth for what exists, who
//! is working on it and what is done.
//!
//! Layout on disk: a small pretty-printed header, then one entry per line,
//! sorted by id. That keeps the file greppable and makes every merge a
//! one-line diff per changed entry.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Windows,
    Macos,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Linux, Os::Windows, Os::Macos];

    pub fn name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Windows => "windows",
            Os::Macos => "macos",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Os::Linux => "Linux",
            Os::Windows => "Windows",
            Os::Macos => "macOS",
        }
    }
}

/// Kernel boundary vs. user-mode library API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Syscall,
    Api,
}

impl Layer {
    pub fn name(self) -> &'static str {
        match self {
            Layer::Syscall => "syscall",
            Layer::Api => "api",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Missing,
    /// An agent is working on it; `claimed_by` says which.
    Claimed,
    /// Exists and returns a plausible failure, but is not specced or tested.
    Stub,
    /// Passes its visible and held-out tests against the real OS.
    Green,
    /// Cannot be made to work for reasons outside our control (see README).
    BlockedByDesign,
}

impl Status {
    pub const ALL: [Status; 5] =
        [Status::Missing, Status::Claimed, Status::Stub, Status::Green, Status::BlockedByDesign];

    pub fn name(self) -> &'static str {
        match self {
            Status::Missing => "missing",
            Status::Claimed => "claimed",
            Status::Stub => "stub",
            Status::Green => "green",
            Status::BlockedByDesign => "blocked-by-design",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|st| st.name() == s)
    }
}

fn is_zero_f64(x: &f64) -> bool {
    *x == 0.0
}

fn is_zero_u32(x: &u32) -> bool {
    *x == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// `<os>/<layer>/<component>/<symbol>`. The four parts below are derived
    /// from it on load and not stored separately.
    pub id: String,
    #[serde(skip, default = "default_os")]
    pub os: Os,
    #[serde(skip, default = "default_layer")]
    pub layer: Layer,
    /// DLL, framework, syscall table, ...
    #[serde(skip)]
    pub component: String,
    #[serde(skip)]
    pub symbol: String,
    /// Call number, where the number is part of the ABI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    /// Share of target-app calls that hit this entry, from the call tracer.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub weight: f64,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tests_visible: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tests_held_out: u32,
    /// Only exists on these architectures (absent = all).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arch: Vec<String>,
    /// Export forwarded to another module (`module.Symbol`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward: Option<String>,
    /// Keys into `Coverage::sources`.
    #[serde(rename = "src")]
    pub sources: Vec<String>,
}

fn default_os() -> Os {
    Os::Linux
}

fn default_layer() -> Layer {
    Layer::Api
}

/// Split an id into os, layer, component and symbol.
pub fn parse_id(id: &str) -> Option<(Os, Layer, &str, &str)> {
    let mut parts = id.splitn(4, '/');
    let os = match parts.next()? {
        "linux" => Os::Linux,
        "windows" => Os::Windows,
        "macos" => Os::Macos,
        _ => return None,
    };
    let layer = match parts.next()? {
        "syscall" => Layer::Syscall,
        "api" => Layer::Api,
        _ => return None,
    };
    let component = parts.next().filter(|c| !c.is_empty())?;
    let symbol = parts.next().filter(|s| !s.is_empty())?;
    Some((os, layer, component, symbol))
}

impl Entry {
    pub fn make_id(os: Os, layer: Layer, component: &str, symbol: &str) -> String {
        format!("{}/{}/{}/{}", os.name(), layer.name(), component, symbol)
    }

    pub fn new(os: Os, layer: Layer, component: &str, symbol: &str, source: &str) -> Entry {
        Entry {
            id: Entry::make_id(os, layer, component, symbol),
            os,
            layer,
            component: component.to_string(),
            symbol: symbol.to_string(),
            number: None,
            status: Status::Missing,
            claimed_by: None,
            weight: 0.0,
            tests_visible: 0,
            tests_held_out: 0,
            arch: Vec::new(),
            forward: None,
            sources: vec![source.to_string()],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub description: String,
    /// Upstream commit or version the import was taken from.
    pub revision: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Traces {
    pub passing: u32,
    pub total: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coverage {
    pub schema_version: u32,
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
    /// Recorded app trace sessions per OS.
    #[serde(default)]
    pub traces: BTreeMap<Os, Traces>,
    pub entries: Vec<Entry>,
}

impl Default for Coverage {
    fn default() -> Self {
        Coverage { schema_version: SCHEMA_VERSION, sources: BTreeMap::new(), traces: BTreeMap::new(), entries: vec![] }
    }
}

impl Coverage {
    pub fn parse(text: &str) -> Result<Coverage, String> {
        let mut c: Coverage = serde_json::from_str(text).map_err(|e| format!("coverage.json: {e}"))?;
        if c.schema_version != SCHEMA_VERSION {
            return Err(format!("unsupported schema_version {}", c.schema_version));
        }
        for e in &mut c.entries {
            let (os, layer, component, symbol) = parse_id(&e.id).ok_or_else(|| format!("malformed id {}", e.id))?;
            (e.os, e.layer) = (os, layer);
            (e.component, e.symbol) = (component.to_string(), symbol.to_string());
        }
        Ok(c)
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("{\n");
        let _ = writeln!(out, "  \"schema_version\": {},", self.schema_version);
        let _ = writeln!(out, "  \"sources\": {},", indent(&serde_json::to_string_pretty(&self.sources).unwrap()));
        let _ = writeln!(out, "  \"traces\": {},", indent(&serde_json::to_string_pretty(&self.traces).unwrap()));
        out.push_str("  \"entries\": [\n");
        for (i, e) in self.entries.iter().enumerate() {
            let sep = if i + 1 == self.entries.len() { "" } else { "," };
            let _ = writeln!(out, "    {}{sep}", serde_json::to_string(e).unwrap());
        }
        out.push_str("  ]\n}\n");
        out
    }

    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// Add or refresh imported entries. Existing entries keep their status,
    /// claim, weight and test counts; only imported facts are updated.
    pub fn merge(&mut self, imported: Vec<Entry>) -> MergeStats {
        let mut index: BTreeMap<String, usize> =
            self.entries.iter().enumerate().map(|(i, e)| (e.id.clone(), i)).collect();
        let mut stats = MergeStats::default();
        for new in imported {
            match index.get(&new.id) {
                Some(&i) => {
                    let e = &mut self.entries[i];
                    e.number = new.number.or(e.number);
                    e.arch = new.arch;
                    e.forward = new.forward;
                    for s in new.sources {
                        if !e.sources.contains(&s) {
                            e.sources.push(s);
                        }
                    }
                    stats.updated += 1;
                }
                None => {
                    index.insert(new.id.clone(), self.entries.len());
                    self.entries.push(new);
                    stats.added += 1;
                }
            }
        }
        self.sort();
        stats
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// Invariants that CI enforces.
    pub fn check(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for w in self.entries.windows(2) {
            if w[0].id >= w[1].id {
                errors.push(format!("entries not sorted / duplicate id at {}", w[1].id));
            }
        }
        for e in &self.entries {
            if e.id != Entry::make_id(e.os, e.layer, &e.component, &e.symbol) {
                errors.push(format!("{}: id does not match os/layer/component/symbol", e.id));
            }
            match (e.status, &e.claimed_by) {
                (Status::Claimed, None) => errors.push(format!("{}: claimed without claimed_by", e.id)),
                (Status::Claimed, Some(_)) => {}
                (_, Some(_)) => errors.push(format!("{}: claimed_by set but status is {}", e.id, e.status.name())),
                _ => {}
            }
            if e.status == Status::Green && e.tests_visible == 0 {
                errors.push(format!("{}: green with no tests", e.id));
            }
            if !e.weight.is_finite() || e.weight < 0.0 {
                errors.push(format!("{}: invalid weight {}", e.id, e.weight));
            }
            if e.sources.is_empty() {
                errors.push(format!("{}: no source", e.id));
            }
            for s in &e.sources {
                if !self.sources.contains_key(s) {
                    errors.push(format!("{}: unknown source {s}", e.id));
                }
            }
        }
        errors
    }
}

#[derive(Debug, Default)]
pub struct MergeStats {
    pub added: usize,
    pub updated: usize,
}

fn indent(s: &str) -> String {
    s.replace('\n', "\n  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Coverage {
        let mut c = Coverage::default();
        c.sources.insert("t".into(), Source { description: "test".into(), revision: "1".into() });
        c.merge(vec![
            Entry::new(Os::Windows, Layer::Api, "kernel32.dll", "CreateFileW", "t"),
            Entry::new(Os::Linux, Layer::Syscall, "x86_64", "read", "t"),
        ]);
        c
    }

    #[test]
    fn roundtrip_is_stable() {
        let c = sample();
        let text = c.to_text();
        let back = Coverage::parse(&text).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.to_text(), text);
    }

    #[test]
    fn merge_keeps_progress() {
        let mut c = sample();
        let id = "windows/api/kernel32.dll/CreateFileW";
        let e = c.find_mut(id).unwrap();
        e.status = Status::Green;
        e.tests_visible = 10;
        let stats = c.merge(vec![Entry::new(Os::Windows, Layer::Api, "kernel32.dll", "CreateFileW", "t")]);
        assert_eq!((stats.added, stats.updated), (0, 1));
        assert_eq!(c.find_mut(id).unwrap().status, Status::Green);
    }

    #[test]
    fn check_catches_bad_claims() {
        let mut c = sample();
        assert!(c.check().is_empty());
        c.entries[0].status = Status::Claimed;
        assert_eq!(c.check().len(), 1);
    }
}
