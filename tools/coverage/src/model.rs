//! The coverage map in `coverage/`: the single source of truth for what
//! exists, who is working on it and what is done.
//!
//! Layout on disk (see ADR 0006):
//!
//! ```text
//! coverage/index.json                        schema version, sources, traces
//! coverage/<os>/<layer>/<component>.json     one entry per line, sorted by symbol
//! ```
//!
//! Small files keep every file well under GitHub's size limits and let many
//! agents claim work in parallel without touching the same file.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
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

    fn is_missing(&self) -> bool {
        *self == Status::Missing
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
    /// `<os>/<layer>/<component>/<symbol>`. On disk only the symbol is
    /// stored; the rest comes from the file's path.
    #[serde(skip)]
    pub id: String,
    #[serde(skip, default = "default_os")]
    pub os: Os,
    #[serde(skip, default = "default_layer")]
    pub layer: Layer,
    /// DLL, framework, syscall table, ...
    #[serde(skip)]
    pub component: String,
    pub symbol: String,
    /// Call number, where the number is part of the ABI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(default, skip_serializing_if = "Status::is_missing")]
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
    /// Read the coverage map from `dir`. A missing directory is an empty map.
    pub fn load(dir: &Path) -> Result<Coverage, String> {
        let index_path = dir.join(INDEX);
        let mut c = match std::fs::read_to_string(&index_path) {
            Ok(text) => {
                let index: Index = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", index_path.display()))?;
                if index.schema_version != SCHEMA_VERSION {
                    return Err(format!("unsupported schema_version {}", index.schema_version));
                }
                Coverage {
                    schema_version: index.schema_version,
                    sources: index.sources,
                    traces: index.traces,
                    entries: vec![],
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Coverage::default(),
            Err(e) => return Err(format!("{}: {e}", index_path.display())),
        };
        for (os, layer, component, path) in component_files(dir)? {
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let entries: Vec<Entry> = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            for mut e in entries {
                e.id = Entry::make_id(os, layer, &component, &e.symbol);
                (e.os, e.layer, e.component) = (os, layer, component.clone());
                c.entries.push(e);
            }
        }
        c.sort();
        Ok(c)
    }

    /// Every file of the on-disk form: (path relative to the coverage dir, contents).
    pub fn files(&self) -> BTreeMap<PathBuf, String> {
        let mut out = BTreeMap::new();
        let index =
            Index { schema_version: self.schema_version, sources: self.sources.clone(), traces: self.traces.clone() };
        out.insert(PathBuf::from(INDEX), serde_json::to_string_pretty(&index).unwrap() + "\n");
        let mut groups: BTreeMap<PathBuf, Vec<&Entry>> = BTreeMap::new();
        for e in &self.entries {
            groups.entry(component_path(e.os, e.layer, &e.component)).or_default().push(e);
        }
        for (path, entries) in groups {
            let lines: Vec<String> = entries.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
            out.insert(path, format!("[\n{}\n]\n", lines.join(",\n")));
        }
        out
    }

    /// Write the map to `dir`, removing component files that no longer exist.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let files = self.files();
        for (_, _, _, path) in component_files(dir)? {
            let rel = path.strip_prefix(dir).unwrap().to_path_buf();
            if !files.contains_key(&rel) {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        for (rel, contents) in files {
            let path = dir.join(&rel);
            if std::fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()) {
                continue;
            }
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    }

    /// Files in `dir` that differ from (or are missing in, or are extra to)
    /// the canonical form of this map.
    pub fn stale_files(&self, dir: &Path) -> Result<Vec<PathBuf>, String> {
        let files = self.files();
        let mut stale = Vec::new();
        for (rel, contents) in &files {
            if std::fs::read_to_string(dir.join(rel)).ok().as_deref() != Some(contents.as_str()) {
                stale.push(rel.clone());
            }
        }
        for (_, _, _, path) in component_files(dir)? {
            let rel = path.strip_prefix(dir).unwrap().to_path_buf();
            if !files.contains_key(&rel) {
                stale.push(rel);
            }
        }
        Ok(stale)
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
            if !valid_component(&e.component) {
                errors.push(format!("{}: component name cannot be a file name", e.id));
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

const INDEX: &str = "index.json";

#[derive(Serialize, Deserialize)]
struct Index {
    schema_version: u32,
    #[serde(default)]
    sources: BTreeMap<String, Source>,
    /// Recorded app trace sessions per OS.
    #[serde(default)]
    traces: BTreeMap<Os, Traces>,
}

fn valid_component(c: &str) -> bool {
    !c.is_empty() && !c.starts_with('.') && !c.contains(['/', '\\']) && c != "index"
}

fn component_path(os: Os, layer: Layer, component: &str) -> PathBuf {
    PathBuf::from(os.name()).join(layer.name()).join(format!("{component}.json"))
}

/// All `<os>/<layer>/<component>.json` files under `dir`, sorted.
fn component_files(dir: &Path) -> Result<Vec<(Os, Layer, String, PathBuf)>, String> {
    let mut out = Vec::new();
    for os in Os::ALL {
        for layer in [Layer::Syscall, Layer::Api] {
            let d = dir.join(os.name()).join(layer.name());
            let Ok(read) = std::fs::read_dir(&d) else { continue };
            let mut names = BTreeSet::new();
            for f in read {
                let f = f.map_err(|e| e.to_string())?;
                let name = f.file_name().to_string_lossy().to_string();
                if let Some(component) = name.strip_suffix(".json") {
                    names.insert(component.to_string());
                }
            }
            for component in names {
                let path = d.join(format!("{component}.json"));
                out.push((os, layer, component, path));
            }
        }
    }
    Ok(out)
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

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("poly-coverage-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn roundtrip_is_stable() {
        let c = sample();
        let dir = temp_dir("roundtrip");
        c.save(&dir).unwrap();
        assert!(dir.join("windows/api/kernel32.dll.json").exists());
        let back = Coverage::load(&dir).unwrap();
        assert_eq!(back, c);
        assert!(back.stale_files(&dir).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_removes_vanished_components_and_check_sees_extras() {
        let mut c = sample();
        let dir = temp_dir("vanish");
        c.save(&dir).unwrap();
        c.entries.retain(|e| e.os != Os::Linux);
        assert_eq!(c.stale_files(&dir).unwrap(), [PathBuf::from("linux/syscall/x86_64.json")]);
        c.save(&dir).unwrap();
        assert!(!dir.join("linux/syscall/x86_64.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_status_is_not_stored() {
        let c = sample();
        let text = &c.files()[&PathBuf::from("windows/api/kernel32.dll.json")];
        assert_eq!(text, "[\n{\"symbol\":\"CreateFileW\",\"src\":[\"t\"]}\n]\n");
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
