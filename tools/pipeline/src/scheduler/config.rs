//! The scheduler's configuration file (TOML).
//!
//! Looked up in this order: `--config FILE`, `$POLY_PIPELINE_CONFIG`,
//! `/etc/poly-pipeline.toml`, `/srv/poly/runner/pipeline.toml` (where
//! setup.sh installs it), `~/.config/poly/pipeline.toml`. Every key has
//! a default suited to the home server layout under `/srv/poly`
//! (docs/AGENT_PIPELINE.md); an example with every key is
//! `tools/pipeline/deploy/pipeline.example.toml`. Unknown keys are errors,
//! so a typo cannot silently fall back to a default.
//!
//! Only the part of TOML a configuration needs is read: `[section]`
//! headers, `key = value` with strings (`"..."` with `\\ \" \n \t`
//! escapes, or `'...'`), integers, booleans and arrays of those (which may
//! span lines), and `#` comments.

use crate::util::{self, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Array(Vec<Value>),
}

/// Parse the TOML subset into `section.key -> value` (keys before any
/// section header have no `section.` prefix).
pub fn parse_toml(text: &str) -> Result<BTreeMap<String, Value>> {
    let mut out = BTreeMap::new();
    let mut section = String::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let n = i + 1;
        let mut line = strip_comment(lines[i]).trim().to_string();
        i += 1;
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.strip_suffix(']').ok_or(format!("line {n}: bad section header"))?.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) {
                return Err(format!("line {n}: bad section name {name:?}"));
            }
            section = name.to_string();
            continue;
        }
        let (key, _) = line.split_once('=').ok_or(format!("line {n}: expected key = value"))?;
        let key = key.trim().to_string();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)) {
            return Err(format!("line {n}: bad key {key:?}"));
        }
        // An array may continue on the following lines.
        while line.split_once('=').unwrap().1.trim_start().starts_with('[') && !brackets_closed(&line) {
            let next = lines.get(i).ok_or(format!("line {n}: unterminated array"))?;
            line.push(' ');
            line.push_str(strip_comment(next).trim());
            i += 1;
        }
        let raw = line.split_once('=').unwrap().1.trim();
        let (value, rest) = parse_value(raw).map_err(|e| format!("line {n}: {e}"))?;
        if !rest.trim().is_empty() {
            return Err(format!("line {n}: unexpected {:?} after the value", rest.trim()));
        }
        let full = if section.is_empty() { key } else { format!("{section}.{key}") };
        if out.insert(full.clone(), value).is_some() {
            return Err(format!("line {n}: {full} is set twice"));
        }
    }
    Ok(out)
}

/// `line` without a `#` comment (a `#` inside a string stays).
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '#' => return &line[..i],
            None => {}
        }
    }
    line
}

/// Whether every `[` outside strings in `s` is closed.
fn brackets_closed(s: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for c in s.chars() {
        match quote {
            Some('"') if escaped => escaped = false,
            Some('"') if c == '\\' => escaped = true,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '[' => depth += 1,
            None if c == ']' => depth -= 1,
            None => {}
        }
    }
    depth <= 0
}

fn parse_value(s: &str) -> Result<(Value, &str)> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Ok((Value::Str(out), &rest[i + 1..])),
                '\\' => match chars.next().map(|(_, c)| c) {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    other => return Err(format!("unsupported escape \\{}", other.unwrap_or(' '))),
                },
                c => out.push(c),
            }
        }
        return Err("unterminated string".into());
    }
    if let Some(rest) = s.strip_prefix('\'') {
        let end = rest.find('\'').ok_or("unterminated string")?;
        return Ok((Value::Str(rest[..end].to_string()), &rest[end + 1..]));
    }
    if let Some(mut rest) = s.strip_prefix('[') {
        let mut items = Vec::new();
        loop {
            rest = rest.trim_start();
            if let Some(r) = rest.strip_prefix(']') {
                return Ok((Value::Array(items), r));
            }
            let (v, r) = parse_value(rest)?;
            items.push(v);
            rest = r.trim_start();
            if let Some(r) = rest.strip_prefix(',') {
                rest = r;
            } else if !rest.starts_with(']') {
                return Err("expected , or ] in an array".into());
            }
        }
    }
    let end = s.find(|c: char| c == ',' || c == ']' || c.is_whitespace()).unwrap_or(s.len());
    let (word, rest) = s.split_at(end);
    match word {
        "true" => Ok((Value::Bool(true), rest)),
        "false" => Ok((Value::Bool(false), rest)),
        w => {
            let n = w.replace('_', "").parse::<i64>().map_err(|_| format!("bad value {w:?}"))?;
            Ok((Value::Int(n), rest))
        }
    }
}

/// One LLM role as the scheduler runs it.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleConfig {
    /// Agent id written to the hand-off log (one per role and model).
    pub agent: String,
    /// The command (no shell) that runs the role's fixed wrapper as the
    /// role's OS user; the scheduler appends `sync ...` or `run ...`.
    pub command: Vec<String>,
    /// Passed to the wrapper as `--model`.
    pub model: Option<String>,
    /// The role's own checkout (read by the scheduler to fetch its branch).
    pub repo: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub max_parallel: usize,
    pub max_cycles: u32,
    pub spec_attempts: u32,
    /// Failed spec checks (CI red, an incomplete spec) that go back to the
    /// spec agent with feedback before the entry goes to the human queue.
    pub spec_cycles: u32,
    /// Consecutive infrastructure errors on one entry before a human gets it.
    pub max_errors: u32,
    /// No new entries while this many pipeline PRs wait for a merge.
    pub max_open_prs: usize,
    /// Merges of main into a PR GitHub reports `dirty` before a human gets
    /// it. Each merge of a pipeline PR makes the other open ones conflict
    /// in README.md, so this should be at least `max_open_prs`.
    pub max_refresh: u32,
    pub os: String,
    pub layer: String,
    /// Only entries whose id starts with one of these (empty: all).
    pub entries: Vec<String>,
    /// Never entries whose id starts with one of these.
    pub exclude: Vec<String>,
    pub idle_secs: u64,
    pub limit_pause_secs: u64,
    pub poll_secs: u64,
    /// Restart with the new code when main changes the loop's own files
    /// (only when the loop runs from `repo`, as the systemd unit does).
    pub self_update: bool,

    pub repo: PathBuf,
    pub worktrees: PathBuf,
    pub hub: PathBuf,
    pub heldout: PathBuf,
    pub inbox: PathBuf,
    /// Feedback for the spec agent (readable by the spec role only).
    pub spec_inbox: PathBuf,
    pub state_dir: PathBuf,
    pub log_file: Option<PathBuf>,
    pub human_queue: PathBuf,
    /// The fingerprint of the deploy files setup.sh installed last.
    pub deploy_stamp: PathBuf,

    pub remote: String,
    pub base_branch: String,
    /// `Name <email>` for the scheduler's own commits (else git's config).
    pub author: Option<String>,
    pub cargo: String,
    pub gh: String,

    pub open_prs: bool,
    pub auto_merge: bool,
    pub push_human_branches: bool,
    pub spec_ci: bool,

    pub spec: RoleConfig,
    pub implementer: RoleConfig,
    /// Paths (prefixes; a trailing `/` means a directory) the implementer may change.
    pub implementer_paths: Vec<String>,
    /// Paths no agent may change, in addition to the built-in list.
    pub extra_forbidden: Vec<String>,
    /// Run with `sh -c` on a human-queue entry or a new PR, with
    /// `POLY_EVENT`, `POLY_ENTRY` and `POLY_MESSAGE` set.
    pub notify_command: Option<String>,
}

pub const DEFAULT_IMPLEMENTER_PATHS: &[&str] =
    &["personalities/", "crates/", "kernel/src/", "Cargo.toml", "Cargo.lock", "kernel/Cargo.toml", "kernel/Cargo.lock"];

/// Where setup.sh installs the configuration.
pub const INSTALLED: &str = "/srv/poly/runner/pipeline.toml";

fn role_command(user: &str, role: &str) -> Vec<String> {
    ["sudo", "-n", "-u", user, "/srv/poly/bin/poly-role", role].iter().map(|s| s.to_string()).collect()
}

impl Default for Config {
    fn default() -> Config {
        Config {
            max_parallel: 1,
            max_cycles: 5,
            spec_attempts: 2,
            spec_cycles: 2,
            max_errors: 8,
            max_open_prs: 10,
            max_refresh: 10,
            os: "linux".into(),
            layer: "syscall".into(),
            entries: Vec::new(),
            exclude: Vec::new(),
            idle_secs: 900,
            limit_pause_secs: 1800,
            poll_secs: 600,
            self_update: true,
            repo: "/srv/poly/runner/repo".into(),
            worktrees: "/srv/poly/runner/work".into(),
            hub: "/srv/poly/git/poly.git".into(),
            heldout: "/srv/poly/heldout".into(),
            inbox: "/srv/poly/inbox".into(),
            spec_inbox: "/srv/poly/spec-inbox".into(),
            state_dir: "/srv/poly/runner/state".into(),
            log_file: Some("/srv/poly/runner/log/pipeline.log".into()),
            human_queue: "/srv/poly/runner/human-queue.md".into(),
            deploy_stamp: "/srv/poly/bin/deployed.sha256".into(),
            remote: "origin".into(),
            base_branch: "main".into(),
            author: None,
            cargo: "cargo".into(),
            gh: "gh".into(),
            open_prs: true,
            auto_merge: true,
            push_human_branches: true,
            spec_ci: true,
            spec: RoleConfig {
                agent: "spec-claude-1".into(),
                command: role_command("polyspec", "spec"),
                model: None,
                repo: "/srv/poly/spec/repo".into(),
            },
            implementer: RoleConfig {
                agent: "impl-claude-1".into(),
                command: role_command("polyimpl", "implementer"),
                model: None,
                repo: "/srv/poly/impl/repo".into(),
            },
            implementer_paths: DEFAULT_IMPLEMENTER_PATHS.iter().map(|s| s.to_string()).collect(),
            extra_forbidden: Vec::new(),
            notify_command: None,
        }
    }
}

struct Keys(BTreeMap<String, Value>);

impl Keys {
    fn take(&mut self, key: &str) -> Option<Value> {
        self.0.remove(key)
    }
    fn str(&mut self, key: &str, into: &mut String) -> Result {
        match self.take(key) {
            None => Ok(()),
            Some(Value::Str(s)) => {
                *into = s;
                Ok(())
            }
            Some(v) => Err(format!("{key}: expected a string, got {v:?}")),
        }
    }
    fn opt_str(&mut self, key: &str, into: &mut Option<String>) -> Result {
        let mut s = into.clone().unwrap_or_default();
        self.str(key, &mut s)?;
        *into = if s.is_empty() { None } else { Some(s) };
        Ok(())
    }
    fn path(&mut self, key: &str, into: &mut PathBuf) -> Result {
        let mut s = into.display().to_string();
        self.str(key, &mut s)?;
        if !Path::new(&s).is_absolute() {
            return Err(format!("{key}: {s:?} must be an absolute path"));
        }
        *into = PathBuf::from(s);
        Ok(())
    }
    fn int<T: TryFrom<i64>>(&mut self, key: &str, into: &mut T) -> Result {
        match self.take(key) {
            None => Ok(()),
            Some(Value::Int(n)) => {
                *into = T::try_from(n).map_err(|_| format!("{key}: {n} is out of range"))?;
                Ok(())
            }
            Some(v) => Err(format!("{key}: expected an integer, got {v:?}")),
        }
    }
    fn bool(&mut self, key: &str, into: &mut bool) -> Result {
        match self.take(key) {
            None => Ok(()),
            Some(Value::Bool(b)) => {
                *into = b;
                Ok(())
            }
            Some(v) => Err(format!("{key}: expected true or false, got {v:?}")),
        }
    }
    fn list(&mut self, key: &str, into: &mut Vec<String>) -> Result {
        match self.take(key) {
            None => Ok(()),
            Some(Value::Array(items)) => {
                *into = items
                    .into_iter()
                    .map(|v| match v {
                        Value::Str(s) => Ok(s),
                        v => Err(format!("{key}: expected strings, got {v:?}")),
                    })
                    .collect::<Result<_>>()?;
                Ok(())
            }
            Some(v) => Err(format!("{key}: expected an array of strings, got {v:?}")),
        }
    }
    fn role(&mut self, section: &str, role: &mut RoleConfig) -> Result {
        self.str(&format!("{section}.agent"), &mut role.agent)?;
        self.list(&format!("{section}.command"), &mut role.command)?;
        self.opt_str(&format!("{section}.model"), &mut role.model)?;
        self.path(&format!("{section}.repo"), &mut role.repo)
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config> {
        let mut k = Keys(parse_toml(text)?);
        let mut c = Config::default();
        k.int("loop.max_parallel", &mut c.max_parallel)?;
        k.int("loop.max_cycles", &mut c.max_cycles)?;
        k.int("loop.spec_attempts", &mut c.spec_attempts)?;
        k.int("loop.spec_cycles", &mut c.spec_cycles)?;
        k.int("loop.max_errors", &mut c.max_errors)?;
        k.int("loop.max_open_prs", &mut c.max_open_prs)?;
        k.int("loop.max_refresh", &mut c.max_refresh)?;
        k.str("loop.os", &mut c.os)?;
        k.str("loop.layer", &mut c.layer)?;
        k.list("loop.entries", &mut c.entries)?;
        k.list("loop.exclude", &mut c.exclude)?;
        k.int("loop.idle_secs", &mut c.idle_secs)?;
        k.int("loop.limit_pause_secs", &mut c.limit_pause_secs)?;
        k.int("loop.poll_secs", &mut c.poll_secs)?;
        k.bool("loop.self_update", &mut c.self_update)?;
        k.path("paths.repo", &mut c.repo)?;
        k.path("paths.worktrees", &mut c.worktrees)?;
        k.path("paths.hub", &mut c.hub)?;
        k.path("paths.heldout", &mut c.heldout)?;
        k.path("paths.inbox", &mut c.inbox)?;
        k.path("paths.spec_inbox", &mut c.spec_inbox)?;
        k.path("paths.state_dir", &mut c.state_dir)?;
        let mut log = c.log_file.as_ref().map(|p| p.display().to_string());
        k.opt_str("paths.log_file", &mut log)?;
        c.log_file = log.map(PathBuf::from);
        k.path("paths.human_queue", &mut c.human_queue)?;
        k.path("paths.deploy_stamp", &mut c.deploy_stamp)?;
        k.str("git.remote", &mut c.remote)?;
        k.str("git.base_branch", &mut c.base_branch)?;
        k.opt_str("git.author", &mut c.author)?;
        k.str("tools.cargo", &mut c.cargo)?;
        k.str("tools.gh", &mut c.gh)?;
        k.bool("github.open_prs", &mut c.open_prs)?;
        k.bool("github.auto_merge", &mut c.auto_merge)?;
        k.bool("github.push_human_branches", &mut c.push_human_branches)?;
        k.role("spec", &mut c.spec)?;
        k.bool("spec.ci", &mut c.spec_ci)?;
        k.role("implementer", &mut c.implementer)?;
        k.list("implementer.allowed_paths", &mut c.implementer_paths)?;
        k.list("safety.extra_forbidden", &mut c.extra_forbidden)?;
        k.opt_str("notify.command", &mut c.notify_command)?;
        if let Some(unknown) = k.0.keys().next() {
            return Err(format!("unknown key {unknown}"));
        }
        c.validate()?;
        Ok(c)
    }

    pub fn load(path: &Path) -> Result<Config> {
        Config::parse(&util::read_string(path)?).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The configuration file to use when none is given: the first that
    /// exists of `$POLY_PIPELINE_CONFIG`, `/etc/poly-pipeline.toml`,
    /// `/srv/poly/runner/pipeline.toml`, `~/.config/poly/pipeline.toml`.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("POLY_PIPELINE_CONFIG").filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        // The second is where setup.sh installs it: `loop status` and
        // `loop pause` work there without naming it.
        let mut candidates = vec![PathBuf::from("/etc/poly-pipeline.toml"), PathBuf::from(INSTALLED)];
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".config/poly/pipeline.toml"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    pub fn validate(&self) -> Result {
        if self.max_parallel == 0 || self.max_parallel > 8 {
            return Err("loop.max_parallel must be between 1 and 8".into());
        }
        if self.max_cycles == 0 || self.spec_attempts == 0 || self.max_errors == 0 {
            return Err("loop.max_cycles, spec_attempts and max_errors must be at least 1".into());
        }
        if self.spec.agent == self.implementer.agent {
            return Err("spec.agent and implementer.agent must differ (CLEAN_ROOM.md, role separation)".into());
        }
        for agent in [&self.spec.agent, &self.implementer.agent] {
            if !valid_name(agent) {
                return Err(format!("agent id {agent:?}: letters, digits, '.', '_' and '-' only"));
            }
        }
        for model in [&self.spec.model, &self.implementer.model].into_iter().flatten() {
            if !valid_name(model) {
                return Err(format!("model {model:?}: letters, digits, '.', '_' and '-' only"));
            }
        }
        if self.spec.command.is_empty() || self.implementer.command.is_empty() {
            return Err("spec.command and implementer.command cannot be empty".into());
        }
        for (name, p) in
            [("worktrees", &self.worktrees), ("inbox", &self.inbox), ("implementer.repo", &self.implementer.repo)]
        {
            if util::is_inside(p, &self.heldout) || util::is_inside(&self.heldout, p) {
                return Err(format!("{name} {} overlaps the held-out store", p.display()));
            }
        }
        if util::is_inside(&self.heldout, &self.repo) || util::is_inside(&self.repo, &self.heldout) {
            return Err("the held-out store overlaps the runner's repository".into());
        }
        // The spec agent's feedback is never where the implementer can read it.
        for (name, p) in [("inbox", &self.inbox), ("implementer.repo", &self.implementer.repo)] {
            if util::is_inside(p, &self.spec_inbox) || util::is_inside(&self.spec_inbox, p) {
                return Err(format!("paths.spec_inbox {} overlaps {name} {}", self.spec_inbox.display(), p.display()));
            }
        }
        Ok(())
    }

    /// Lines for `loop status`.
    pub fn describe(&self) -> String {
        format!(
            "runner {} (hub {}), state {}, held-out {}, max_parallel {}, max_cycles {}, spec_cycles {}, PRs {}{}",
            self.repo.display(),
            self.hub.display(),
            self.state_dir.display(),
            self.heldout.display(),
            self.max_parallel,
            self.max_cycles,
            self.spec_cycles,
            if self.open_prs { "on" } else { "off" },
            if self.open_prs && self.auto_merge { " with auto-merge" } else { "" }
        )
    }
}

/// Agent ids, models: no spaces or shell characters.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_subset() {
        let t = parse_toml(
            "top = 1 # comment\n[a]\ns = \"x # y \\\"q\\\"\"\nl = 'lit\\eral'\nb = false\nn = 1_000\n\
             arr = [\n  \"one\", # first\n  \"two\",\n]\n[b.c]\nempty = []\n",
        )
        .unwrap();
        assert_eq!(t["top"], Value::Int(1));
        assert_eq!(t["a.s"], Value::Str("x # y \"q\"".into()));
        assert_eq!(t["a.l"], Value::Str("lit\\eral".into()));
        assert_eq!(t["a.b"], Value::Bool(false));
        assert_eq!(t["a.n"], Value::Int(1000));
        assert_eq!(t["a.arr"], Value::Array(vec![Value::Str("one".into()), Value::Str("two".into())]));
        assert_eq!(t["b.c.empty"], Value::Array(vec![]));
        assert!(parse_toml("x = 1\nx = 2\n").unwrap_err().contains("twice"));
        assert!(parse_toml("x = \"open\n").is_err());
        assert!(parse_toml("x = [1,\n").is_err());
        assert!(parse_toml("x = 1 2\n").is_err());
        assert!(parse_toml("just words\n").is_err());
    }

    #[test]
    fn config_defaults_and_overrides() {
        let c = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!((c.max_cycles, c.spec_cycles), (5, 2));
        let c = Config::parse(
            "[loop]\nmax_parallel = 2\nentries = [\"linux/syscall/x86_64/\"]\n[spec]\nagent = \"spec-qwen-1\"\n\
             model = \"opus\"\n[implementer]\ncommand = [\"/bin/true\"]\n[github]\nauto_merge = false\n\
             [paths]\nlog_file = \"\"\n",
        )
        .unwrap();
        assert_eq!(c.max_parallel, 2);
        assert_eq!(c.entries, ["linux/syscall/x86_64/"]);
        assert_eq!(c.spec.agent, "spec-qwen-1");
        assert_eq!(c.spec.model.as_deref(), Some("opus"));
        assert_eq!(c.implementer.command, ["/bin/true"]);
        assert!(!c.auto_merge && c.open_prs);
        assert_eq!(c.log_file, None);
    }

    #[test]
    fn config_errors() {
        assert!(Config::parse("[loop]\nmax_paralel = 2\n").unwrap_err().contains("unknown key loop.max_paralel"));
        assert!(Config::parse("[loop]\nmax_parallel = \"2\"\n").unwrap_err().contains("integer"));
        assert!(Config::parse("[spec]\nagent = \"impl-claude-1\"\n").unwrap_err().contains("must differ"));
        assert!(Config::parse("[paths]\nrepo = \"relative\"\n").unwrap_err().contains("absolute"));
        assert!(Config::parse("[paths]\ninbox = \"/srv/poly/heldout/inbox\"\n").unwrap_err().contains("overlaps"));
        assert!(Config::parse("[paths]\nspec_inbox = \"/srv/poly/inbox/spec\"\n").unwrap_err().contains("spec_inbox"));
        assert!(Config::parse("[spec]\nagent = \"a b\"\n").is_err());
        assert!(Config::parse("[loop]\nmax_parallel = 0\n").is_err());
    }

    #[test]
    fn the_example_file_parses() {
        let text = util::read_string(&crate::repo_root().join("tools/pipeline/deploy/pipeline.example.toml")).unwrap();
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.max_parallel, 2);
        assert_eq!((c.spec_cycles, c.spec_inbox.to_str()), (2, Some("/srv/poly/spec-inbox")));
        assert_eq!(c.spec.command[..4], ["sudo", "-n", "-u", "polyspec"]);
        assert_eq!(c.implementer.command[..4], ["sudo", "-n", "-u", "polyimpl"]);
        assert!(c.self_update && c.deploy_stamp.to_str() == Some("/srv/poly/bin/deployed.sha256"));
        assert!(Config::parse("[loop]\nself_update = false\n").is_ok_and(|c| !c.self_update));
    }
}
