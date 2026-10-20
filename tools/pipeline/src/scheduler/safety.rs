//! Safety checks on what agents changed (docs/AGENT_PIPELINE.md, "Safety
//! rules"), run on `git diff` output before the loop runs anything from a
//! branch and again on the whole PR before it is opened.
//!
//! - Every changed path must be one the role may change: the spec agent
//!   only its entry's spec sheet and visible tests, the implementer only
//!   the configured implementation paths.
//! - No agent changes the files the project's rules reserve
//!   ([`FORBIDDEN`]): the roadmap, the README (its progress section is
//!   generated), `docs/progress/`, `coverage/`, CLEAN_ROOM.md, the hand-off
//!   log, the tools that judge the work, CI configuration.
//! - No added `unsafe` (agent-written code is safe Rust only), no build
//!   scripts, no new crates from a registry or git, no binary files.
//! - The PR as a whole may only add the loop's own changes on top: the
//!   hand-off log rows of this entry, this entry's coverage status, and the
//!   generated README progress section and graphics.
//!
//! Any problem stops the entry and puts it in the human queue.

use crate::store::EntryId;
use serde_json::Value;

/// Paths no agent may change (a trailing `/` means everything below).
pub const FORBIDDEN: &[&str] = &[
    "docs/ROADMAP.md",
    "README.md",
    "docs/progress/",
    "coverage/",
    "CLEAN_ROOM.md",
    "CLAUDE.md",
    "specs/HANDOFF_LOG.md",
    "specs/TEMPLATE.yaml",
    "specs/README.md",
    "tools/",
    "tests/",
    ".github/",
    ".claude/",
    ".cargo/",
    "rust-toolchain.toml",
    "LICENSE",
    "THIRD_PARTY.md",
    "third_party/",
];

/// One file in a diff.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// Added lines, without the leading `+`.
    pub added: Vec<String>,
    /// Removed lines, without the leading `-`.
    pub removed: Vec<String>,
    pub binary: bool,
    /// Git had to quote the name (unusual characters).
    pub quoted: bool,
}

/// Parse `git diff --no-renames -U0` output.
pub fn parse_diff(text: &str) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let quoted = rest.starts_with('"');
            // With --no-renames both sides name the same file: "a/P b/P".
            let path = if quoted {
                rest.to_string()
            } else {
                let half = rest.len().saturating_sub(1) / 2;
                rest.get(half + 1..).and_then(|b| b.strip_prefix("b/")).unwrap_or(rest).to_string()
            };
            out.push(Change { path, quoted, ..Change::default() });
            continue;
        }
        let Some(c) = out.last_mut() else { continue };
        if line.starts_with("Binary files ") || line == "GIT binary patch" {
            c.binary = true;
        } else if line.starts_with("+++ ") || line.starts_with("--- ") {
            // File names: taken from the header above.
        } else if let Some(a) = line.strip_prefix('+') {
            c.added.push(a.to_string());
        } else if let Some(r) = line.strip_prefix('-') {
            c.removed.push(r.to_string());
        }
    }
    out
}

/// Whether `path` is `rule` or below it (`rule` ending in `/`).
pub fn matches(path: &str, rule: &str) -> bool {
    if rule.ends_with('/') { path.starts_with(rule) } else { path == rule }
}

fn word_at(line: &str, word: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(i) = line[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before = line[..start].chars().next_back();
        let after = line[end..].chars().next();
        if !before.is_some_and(ident) && !after.is_some_and(ident) {
            return true;
        }
        from = end;
    }
    false
}

/// Problems in the added lines of one file, whatever role changed it.
fn content_problems(c: &Change) -> Vec<String> {
    let mut out = Vec::new();
    if c.quoted {
        out.push(format!("{}: unusual file name", c.path));
    }
    if c.binary {
        out.push(format!("{}: binary file (origin cannot be reviewed)", c.path));
    }
    let name = c.path.rsplit('/').next().unwrap_or(&c.path);
    if name == "build.rs" {
        out.push(format!("{}: build scripts run on the runner; agents may not add or change them", c.path));
    }
    if c.path.ends_with(".rs") {
        for l in &c.added {
            let code = l.split("//").next().unwrap_or_default();
            if word_at(code, "unsafe") || code.contains("unsafe_code") || code.contains("global_asm!") {
                out.push(format!("{}: adds unsafe code (agent code is safe Rust only): {}", c.path, l.trim()));
            }
        }
    }
    if name == "Cargo.lock" && c.added.iter().any(|l| l.trim_start().starts_with("source = ")) {
        out.push(format!("{}: adds a crate from a registry or git (third-party crates need the owner)", c.path));
    }
    if name == "Cargo.toml" && c.added.iter().any(|l| l.contains("proc-macro") || l.contains("build =")) {
        out.push(format!("{}: adds a build script or proc-macro crate", c.path));
    }
    out
}

/// The paths the spec agent may change for `entry`.
pub fn spec_paths(entry: &EntryId) -> Vec<String> {
    vec![entry.spec_path().display().to_string(), format!("{}/", entry.tests_dir().display())]
}

/// What an agent changed in its own commits (`role` names it in messages).
pub fn check_role(role: &str, changes: &[Change], allowed: &[String], extra_forbidden: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for c in changes {
        if let Some(rule) = FORBIDDEN.iter().copied().chain(extra_forbidden.iter().map(String::as_str)).find(|r| {
            matches(&c.path, r) && !allowed.iter().any(|a| a.len() > r.len() && matches(&c.path, a) && a.starts_with(r))
        }) {
            out.push(format!("{}: the {role} may not change {rule}", c.path));
        } else if !allowed.iter().any(|a| matches(&c.path, a)) {
            out.push(format!("{}: outside the paths the {role} may change", c.path));
        }
        out.extend(content_problems(c));
    }
    out
}

/// Coverage files are one JSON object per line: compare the entry's line.
fn coverage_problems(entry: &EntryId, before: Option<&str>, after: Option<&str>) -> Vec<String> {
    let parse = |text: Option<&str>| -> Result<Vec<Value>, String> {
        let text = text.ok_or("missing")?;
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        v.as_array().cloned().ok_or_else(|| "not an array".to_string())
    };
    let (Ok(before), Ok(after)) = (parse(before), parse(after)) else {
        return vec!["the entry's coverage file is missing or not JSON".into()];
    };
    let symbol = |v: &Value| v.get("symbol").and_then(Value::as_str).map(String::from);
    let mut out = Vec::new();
    if before.len() != after.len() {
        out.push("the coverage file gained or lost entries".into());
        return out;
    }
    for (b, a) in before.iter().zip(&after) {
        if symbol(b) != symbol(a) {
            out.push("the coverage file's entries were reordered".into());
            break;
        }
        if b == a {
            continue;
        }
        if symbol(a).as_deref() != Some(entry.symbol.as_str()) {
            out.push(format!("coverage entry {} changed (only {} may)", symbol(a).unwrap_or_default(), entry.id()));
            continue;
        }
        let (bo, ao) = (b.as_object().cloned().unwrap_or_default(), a.as_object().cloned().unwrap_or_default());
        let status_fields = ["status", "claimed_by", "tests_visible", "tests_held_out"];
        for k in bo.keys().chain(ao.keys()) {
            if !status_fields.contains(&k.as_str()) && bo.get(k) != ao.get(k) {
                out.push(format!("coverage field {k} of {} changed", entry.id()));
            }
        }
        if a.get("status").and_then(Value::as_str) != Some("green") {
            out.push(format!("{} is not green in the coverage map", entry.id()));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Inputs of [`check_pr`].
#[derive(Debug)]
pub struct PrCheck<'a> {
    pub entry: &'a EntryId,
    /// The whole PR: merge base with main to the head.
    pub changes: &'a [Change],
    pub implementer_paths: &'a [String],
    pub extra_forbidden: &'a [String],
    /// The entry's coverage file at the merge base and at the head.
    pub coverage_before: Option<&'a str>,
    pub coverage_after: Option<&'a str>,
}

/// The coverage file of `entry`, relative to the repository root.
pub fn coverage_file(entry: &EntryId) -> String {
    format!("coverage/{}/{}/{}.json", entry.os, entry.layer, entry.component)
}

/// The whole PR: agent paths plus the loop's own status changes, nothing else.
pub fn check_pr(p: &PrCheck<'_>) -> Vec<String> {
    let coverage = coverage_file(p.entry);
    let mut out = Vec::new();
    let spec = spec_paths(p.entry);
    for c in p.changes {
        out.extend(content_problems(c));
        if c.path == coverage {
            out.extend(coverage_problems(p.entry, p.coverage_before, p.coverage_after));
        } else if c.path == "specs/HANDOFF_LOG.md" {
            if !c.removed.is_empty() {
                out.push("specs/HANDOFF_LOG.md: rows were changed or removed (the log is append-only)".into());
            }
            for row in &c.added {
                let entry_cell = row.trim().trim_matches('|').split(" | ").nth(1).map(str::trim);
                if entry_cell != Some(p.entry.id().as_str()) {
                    out.push(format!("specs/HANDOFF_LOG.md: added a row that is not a hand-off of {}", p.entry.id()));
                }
            }
        } else if c.path == "README.md" || matches(&c.path, "docs/progress/") {
            // Generated by `coverage readme` from the status change;
            // `coverage readme --check` proves it.
        } else if !spec.iter().chain(p.implementer_paths).any(|a| matches(&c.path, a))
            || FORBIDDEN.iter().copied().chain(p.extra_forbidden.iter().map(String::as_str)).any(|r| {
                matches(&c.path, r)
                    && !spec.iter().chain(p.implementer_paths).any(|a| a.len() > r.len() && matches(&c.path, a))
            })
        {
            out.push(format!("{}: not a path this PR may change", c.path));
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/personalities/linux/src/brk.rs b/personalities/linux/src/brk.rs
index 1..2 100644
--- a/personalities/linux/src/brk.rs
+++ b/personalities/linux/src/brk.rs
@@ -1,0 +2,2 @@
+pub fn brk() {} // not unsafe at all
+let x = 1;
diff --git a/docs/ROADMAP.md b/docs/ROADMAP.md
--- a/docs/ROADMAP.md
+++ b/docs/ROADMAP.md
@@ -3 +3 @@
-- [ ] brk
+- [x] brk
diff --git a/crates/x/logo.png b/crates/x/logo.png
Binary files /dev/null and b/crates/x/logo.png differ
diff --git a/crates/x/src/gone.rs b/crates/x/src/gone.rs
deleted file mode 100644
--- a/crates/x/src/gone.rs
+++ /dev/null
@@ -1 +0,0 @@
-unsafe { old() }
";

    fn entry() -> EntryId {
        EntryId::parse("linux/syscall/x86_64/brk").unwrap()
    }

    fn implementer_paths() -> Vec<String> {
        crate::scheduler::config::DEFAULT_IMPLEMENTER_PATHS.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_git_diffs() {
        let c = parse_diff(DIFF);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].path, "personalities/linux/src/brk.rs");
        assert_eq!(c[0].added, ["pub fn brk() {} // not unsafe at all", "let x = 1;"]);
        assert_eq!(c[1].path, "docs/ROADMAP.md");
        assert_eq!((c[1].removed.len(), c[1].added.len()), (1, 1));
        assert!(c[2].binary);
        assert_eq!(c[3].path, "crates/x/src/gone.rs");
        assert_eq!(c[3].removed, ["unsafe { old() }"]);
        assert!(parse_diff("diff --git \"a/x\\ty\" \"b/x\\ty\"\n")[0].quoted);
    }

    #[test]
    fn implementer_rules() {
        let p = check_role("implementer", &parse_diff(DIFF), &implementer_paths(), &[]);
        assert_eq!(
            p,
            [
                "docs/ROADMAP.md: the implementer may not change docs/ROADMAP.md",
                "crates/x/logo.png: binary file (origin cannot be reviewed)",
            ],
            "a comment mentioning unsafe and removing unsafe are fine"
        );
        let unsafe_diff = "diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs\n+    let p = unsafe { *ptr };\n\
                           +#![allow(unsafe_code)]\n+let not_unsafe_name = 1;\n";
        let p = check_role("implementer", &parse_diff(unsafe_diff), &implementer_paths(), &[]);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p.iter().all(|m| m.contains("adds unsafe code")));
        let other = "diff --git a/tools/pipeline/src/main.rs b/tools/pipeline/src/main.rs\n+x\n\
                     diff --git a/docs/adr/0099-x.md b/docs/adr/0099-x.md\n+x\n\
                     diff --git a/crates/a/build.rs b/crates/a/build.rs\n+fn main() {}\n\
                     diff --git a/Cargo.lock b/Cargo.lock\n+source = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
        let p = check_role("implementer", &parse_diff(other), &implementer_paths(), &["crates/secret/".into()]);
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(p[0].contains("may not change tools/"));
        assert!(p[1].contains("outside the paths"));
        assert!(p[2].contains("build scripts"));
        assert!(p[3].contains("registry"));
        let p = check_role(
            "implementer",
            &parse_diff("diff --git a/crates/secret/a b/crates/secret/a\n"),
            &implementer_paths(),
            &["crates/secret/".into()],
        );
        assert_eq!(p, ["crates/secret/a: the implementer may not change crates/secret/"]);
    }

    #[test]
    fn spec_rules() {
        let e = entry();
        let ok = "diff --git a/specs/linux/x86_64/brk.yaml b/specs/linux/x86_64/brk.yaml\n+id: x\n\
                  diff --git a/specs/linux/x86_64/brk.tests/t1.c b/specs/linux/x86_64/brk.tests/t1.c\n+int x;\n";
        assert!(check_role("spec agent", &parse_diff(ok), &spec_paths(&e), &[]).is_empty());
        let bad = "diff --git a/specs/linux/x86_64/mmap.yaml b/specs/linux/x86_64/mmap.yaml\n+x\n\
                   diff --git a/specs/HANDOFF_LOG.md b/specs/HANDOFF_LOG.md\n+| row |\n\
                   diff --git a/coverage/linux/syscall/x86_64.json b/coverage/linux/syscall/x86_64.json\n+x\n\
                   diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs\n+x\n";
        let p = check_role("spec agent", &parse_diff(bad), &spec_paths(&e), &[]);
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(p[1].contains("may not change specs/HANDOFF_LOG.md") && p[2].contains("may not change coverage/"));
    }

    fn coverage(brk: &str) -> String {
        format!(
            "[\n{{\"symbol\":\"access\",\"number\":21,\"src\":[\"t\"]}},\n{brk},\n{{\"symbol\":\"mmap\",\"src\":[\"t\"]}}\n]\n"
        )
    }

    #[test]
    fn pr_rules() {
        let e = entry();
        let before = coverage("{\"symbol\":\"brk\",\"number\":12,\"src\":[\"t\"]}");
        let after = coverage(
            "{\"symbol\":\"brk\",\"number\":12,\"status\":\"green\",\"tests_visible\":7,\"tests_held_out\":3,\"src\":[\"t\"]}",
        );
        let diff = "diff --git a/coverage/linux/syscall/x86_64.json b/coverage/linux/syscall/x86_64.json\n-x\n+y\n\
                    diff --git a/README.md b/README.md\n+x\n\
                    diff --git a/docs/progress/linux.svg b/docs/progress/linux.svg\n+x\n\
                    diff --git a/specs/HANDOFF_LOG.md b/specs/HANDOFF_LOG.md\n\
                    +| 2026-10-20 10:00:00 | linux/syscall/x86_64/brk | abc | s | i | `f` sha256:00 |\n\
                    diff --git a/specs/linux/x86_64/brk.yaml b/specs/linux/x86_64/brk.yaml\n+x\n\
                    diff --git a/personalities/linux/src/mm.rs b/personalities/linux/src/mm.rs\n+x\n";
        let changes = parse_diff(diff);
        let paths = implementer_paths();
        let mut check = PrCheck {
            entry: &e,
            changes: &changes,
            implementer_paths: &paths,
            extra_forbidden: &[],
            coverage_before: Some(&before),
            coverage_after: Some(&after),
        };
        assert_eq!(check_pr(&check), Vec::<String>::new());

        let other = coverage("{\"symbol\":\"brk\",\"number\":13,\"status\":\"green\",\"src\":[\"t\"]}")
            .replace("\"number\":21", "\"number\":22");
        check.coverage_after = Some(&other);
        let p = check_pr(&check);
        assert_eq!(
            p,
            [
                "coverage entry access changed (only linux/syscall/x86_64/brk may)",
                "coverage field number of linux/syscall/x86_64/brk changed"
            ]
        );
        let not_green =
            coverage("{\"symbol\":\"brk\",\"number\":12,\"status\":\"claimed\",\"claimed_by\":\"x\",\"src\":[\"t\"]}");
        check.coverage_after = Some(&not_green);
        assert_eq!(check_pr(&check), ["linux/syscall/x86_64/brk is not green in the coverage map"]);
        check.coverage_after = Some(&after);

        let extra = parse_diff(
            "diff --git a/specs/HANDOFF_LOG.md b/specs/HANDOFF_LOG.md\n-| old |\n+| d | linux/syscall/x86_64/mmap | r | s | i | f |\n\
             diff --git a/CLEAN_ROOM.md b/CLEAN_ROOM.md\n+x\ndiff --git a/docs/ROADMAP.md b/docs/ROADMAP.md\n+x\n",
        );
        check.changes = &extra;
        let p = check_pr(&check);
        assert_eq!(p.len(), 4, "{p:?}");
    }
}
