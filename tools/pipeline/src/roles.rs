//! Running an LLM agent in one of the two pipeline roles. The pipeline
//! does not talk to any model itself: each role is a shell command,
//! configured per role, that runs an agent CLI non-interactively in the
//! role's own working directory with a generated prompt.
//!
//! ```text
//! POLY_SPEC_AGENT_CMD   command for the spec agent    (else --preset)
//! POLY_IMPLEMENTER_CMD  command for the implementer   (else --preset)
//! ```
//!
//! Placeholders, substituted shell-quoted: `{prompt_file}`, `{workdir}`,
//! `{entry}`, `{agent}`, `{role}`. The command runs with `sh -c` in the
//! working directory and gets `POLY_ROLE`, `POLY_ENTRY`, `POLY_AGENT_ID`
//! and `POLY_PROMPT_FILE`; the implementer never gets `POLY_HELDOUT_DIR`.
//! Every run is a fresh agent session (no resume), so contexts never mix.
//!
//! When the agent's output says a usage limit was reached (Claude's 5-hour
//! and weekly windows, provider rate limits), the run ends with
//! `LIMIT_EXIT` so a scheduler can pause and retry later.

use crate::store::{self, EntryId};
use crate::util::{self, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Exit status of `pipeline role` when a usage limit was hit (EX_TEMPFAIL).
pub const LIMIT_EXIT: u8 = 75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Spec,
    Implementer,
}

impl Role {
    pub fn parse(s: &str) -> Result<Role> {
        match s {
            "spec" => Ok(Role::Spec),
            "implementer" => Ok(Role::Implementer),
            _ => Err(format!("unknown role {s} (spec, implementer)")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Spec => "spec",
            Role::Implementer => "implementer",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Role::Spec => "POLY_SPEC_AGENT_CMD",
            Role::Implementer => "POLY_IMPLEMENTER_CMD",
        }
    }
}

/// Built-in commands. `model` is passed through when given.
pub fn preset(name: &str, role: Role, model: Option<&str>) -> Result<String> {
    match name {
        // Claude Code in headless mode (`claude -p`), logged in with the
        // owner's Claude subscription or an API key. Tools are allow-listed
        // per role; the implementer cannot reach the web, trace the host
        // or run the pipeline's oracle commands.
        "claude" => {
            let model = model.map_or(String::new(), |m| format!(" --model {}", util::shell_quote(m)));
            let tools = match role {
                Role::Spec => "--allowedTools 'Read Write Edit Glob Grep Bash'",
                Role::Implementer => {
                    "--allowedTools 'Read Write Edit Glob Grep Bash(cargo build:*) Bash(cargo test:*) \
                     Bash(cargo clippy:*) Bash(cargo fmt:*) Bash(cargo xtask ci) Bash(cargo xtask test) \
                     Bash(cargo xtask build) Bash(git status:*) Bash(git diff:*) Bash(git add:*) \
                     Bash(git commit:*) Bash(git log:*)' \
                     --disallowedTools 'WebFetch WebSearch Bash(cargo xtask pipeline:*) Bash(strace:*) \
                     Bash(gdb:*) Bash(objdump:*)'"
                }
            };
            Ok(format!("claude -p --output-format text --permission-mode acceptEdits{model} {tools} < {{prompt_file}}"))
        }
        // OpenCode with any provider/model (`opencode run`). Tool
        // permissions live in the role directory's opencode.json.
        "opencode" => {
            let model = model.map_or(String::new(), |m| format!(" --model {}", util::shell_quote(m)));
            Ok(format!("opencode run{model} \"$(cat {{prompt_file}})\""))
        }
        _ => Err(format!("unknown preset {name} (claude, opencode)")),
    }
}

/// Replace the placeholders in `template`, quoting each value.
pub fn render(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), &util::shell_quote(v));
    }
    out
}

const SPEC_PROMPT: &str = "\
You are the SPEC AGENT of the Poly project for one coverage entry: {entry}.
Your agent id is {agent}. Read CLEAN_ROOM.md, specs/README.md and specs/TEMPLATE.yaml first.

Goal: describe the observable behaviour of {entry} on the real OS, as a spec sheet and tests.
1. Observe the real OS only: run programs and trace them (`cargo xtask pipeline trace ...`),
   read man pages and public headers. You may read Linux source for behaviour, but never copy
   code, comments or structure from it.
2. `cargo xtask pipeline spec new {entry} --agent {agent}` creates the skeleton {spec}.
   Fill it with behaviour only: results, errors, ordering, side effects, timing envelopes.
   No code, no internals.
3. Write test programs (in the style of tests/linux/: freestanding C, machine-readable
   `ok`/`not ok` lines) into {generated}, which is OUTSIDE the repository. Write enough for a
   30% held-out share to matter (10 or more).
4. `cargo xtask pipeline tests split {entry} {generated} --spec-agent {agent}` keeps about 30%
   held out (outside the repository) and copies the rest to {tests}.
5. Commit the spec sheet and visible tests on the current branch (do not create or switch
   branches, do not rewrite commits, never push). Change nothing else in the repository.
Never write implementation code. Never look at implementer output for this entry.
";

const IMPLEMENTER_PROMPT: &str = "\
You are the IMPLEMENTER AGENT of the Poly project for one coverage entry: {entry}.
Your agent id is {agent}. Read CLAUDE.md and CLEAN_ROOM.md first and follow them.

Your only behavioural inputs are the spec sheet {spec} and the visible tests in {tests}
(handed over at revision {revision}), plus public documentation (man pages) and public
headers for constants. You must NOT run, trace or probe the real OS, read any held-out test,
read Linux or any other kernel's source, or copy code from anywhere.

Implement {entry} in the Linux personality (personalities/linux) and the kernel glue it needs,
host-testable logic in crates with unit tests derived from the visible tests. Safe Rust only:
no `unsafe` in code you write, no build scripts, no new third-party crates. Run `cargo xtask ci`
until it passes, then commit on the current branch (do not create or switch branches, do not
rewrite commits). Never push and never edit specs/, coverage/, tools/, tests/, README.md or docs/.
";

#[derive(Debug)]
pub struct Request<'a> {
    pub role: Role,
    pub entry: &'a EntryId,
    pub agent: &'a str,
    /// The role's own directory: a checkout the agent works in.
    pub workdir: &'a Path,
    /// The command template (from the environment or a preset).
    pub command: &'a str,
    /// Where the spec agent writes generated tests (spec role).
    pub generated: Option<&'a Path>,
    /// Revision of the hand-off (implementer role).
    pub revision: Option<&'a str>,
    /// Diff runner output of the previous red cycle, appended to the prompt.
    pub feedback: Option<String>,
    pub dry_run: bool,
}

/// The prompt for `req`.
pub fn prompt(req: &Request<'_>) -> String {
    let base = match req.role {
        Role::Spec => SPEC_PROMPT,
        Role::Implementer => IMPLEMENTER_PROMPT,
    };
    let generated = req.generated.map_or("<a directory outside the repository>".into(), |g| g.display().to_string());
    let mut text = base
        .replace("{entry}", &req.entry.id())
        .replace("{agent}", req.agent)
        .replace("{spec}", &req.entry.spec_path().display().to_string())
        .replace("{tests}", &req.entry.tests_dir().display().to_string())
        .replace("{generated}", &generated)
        .replace("{revision}", req.revision.unwrap_or("HEAD"));
    if let Some(f) = &req.feedback {
        text.push_str(match req.role {
            // The scheduler's spec check failed: the agent continues its own work.
            Role::Spec => {
                "\nThis session continues your previous one, whose branch failed the runner's spec check. \
                 Your commits, the generated tests and the held-out tests are as you left them: do not start \
                 over (the skeleton exists already). Fix what the check found, below; if you change tests, \
                 change them in the generated directory and run `tests split` again with `--replace`. Then \
                 commit on the current branch.\n\n"
            }
            Role::Implementer => "\nThe diff runner found these differences in your previous attempt; fix them:\n\n",
        });
        text.push_str(f);
        text.push('\n');
    }
    text
}

/// Lower-case substrings that mean "usage limit reached, try later".
pub fn limit_patterns() -> Vec<String> {
    match std::env::var("POLY_LIMIT_PATTERNS") {
        Ok(p) if !p.is_empty() => p.split('|').map(|s| s.to_lowercase()).collect(),
        _ => ["usage limit", "limit reached", "rate limit", "rate_limit", "too many requests", "quota exceeded"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    }
}

pub fn hit_limit(output: &str, patterns: &[String]) -> bool {
    let lower = output.to_lowercase();
    patterns.iter().any(|p| lower.contains(p.as_str()))
}

#[derive(Debug)]
pub struct Outcome {
    pub success: bool,
    pub limit_reached: bool,
    pub log: PathBuf,
}

/// Run the role's command. Output is shown and kept in `logs/`.
pub fn run(repo: &Path, logs: &Path, req: &Request<'_>) -> Result<Outcome> {
    let workdir = util::absolute(req.workdir);
    if !workdir.is_dir() {
        return Err(format!("working directory {} does not exist", workdir.display()));
    }
    if req.role == Role::Implementer
        && let Ok(heldout) = store::heldout_root(repo)
        && (util::is_inside(&workdir, &heldout) || util::is_inside(&heldout, &workdir))
    {
        return Err("the implementer's working directory overlaps the held-out store".into());
    }
    let dir = logs.join(format!("{}-{}-{}", util::now_stamp(), req.role.name(), req.entry.symbol));
    let prompt_file = dir.join("prompt.md");
    util::write(&prompt_file, prompt(req))?;
    let vars = [
        ("prompt_file", prompt_file.display().to_string()),
        ("workdir", workdir.display().to_string()),
        ("entry", req.entry.id()),
        ("agent", req.agent.to_string()),
        ("role", req.role.name().to_string()),
    ];
    let command = render(req.command, &vars);
    util::write(&dir.join("command.sh"), format!("{command}\n"))?;
    eprintln!("==> {} agent {} on {} in {}", req.role.name(), req.agent, req.entry.id(), workdir.display());
    eprintln!("    $ {command}");
    let log = dir.join("output.log");
    if req.dry_run {
        eprintln!("    (dry run; prompt in {})", prompt_file.display());
        return Ok(Outcome { success: true, limit_reached: false, log });
    }
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&workdir)
        .env("POLY_ROLE", req.role.name())
        .env("POLY_ENTRY", req.entry.id())
        .env("POLY_AGENT_ID", req.agent)
        .env("POLY_PROMPT_FILE", &prompt_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if req.role == Role::Implementer {
        cmd.env_remove(store::HELDOUT_ENV);
    }
    let mut child = cmd.spawn().map_err(|e| format!("sh: {e}"))?;
    let mut file = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut output = String::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(std::result::Result::ok) {
        println!("{line}");
        let _ = writeln!(file, "{line}");
        output.push_str(&line);
        output.push('\n');
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let limit_reached = hit_limit(&output, &limit_patterns());
    Ok(Outcome { success: status.success() && !limit_reached, limit_reached, log })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> EntryId {
        EntryId::parse("linux/syscall/x86_64/brk").unwrap()
    }

    #[test]
    fn presets_render() {
        let c = preset("claude", Role::Implementer, Some("opus")).unwrap();
        assert!(c.starts_with("claude -p ") && c.contains("--model opus") && c.ends_with("< {prompt_file}"));
        assert!(c.contains("--disallowedTools 'WebFetch WebSearch Bash(cargo xtask pipeline:*)"));
        let s = preset("claude", Role::Spec, None).unwrap();
        assert!(!s.contains("--model") && !s.contains("disallowed"));
        let o = preset("opencode", Role::Spec, Some("ollama/qwen3")).unwrap();
        assert_eq!(o, "opencode run --model ollama/qwen3 \"$(cat {prompt_file})\"");
        assert!(preset("other", Role::Spec, None).is_err());
        let r = render(&o, &[("prompt_file", "/tmp/a b/prompt.md".into())]);
        assert_eq!(r, "opencode run --model ollama/qwen3 \"$(cat '/tmp/a b/prompt.md')\"");
    }

    fn request<'a>(role: Role, e: &'a EntryId, workdir: &'a Path, command: &'a str) -> Request<'a> {
        Request {
            role,
            entry: e,
            agent: "agent-7",
            workdir,
            command,
            generated: None,
            revision: Some("abc123"),
            feedback: None,
            dry_run: false,
        }
    }

    #[test]
    fn prompts_name_the_inputs() {
        let e = entry();
        let mut req = request(Role::Implementer, &e, Path::new("."), "true");
        req.feedback = Some("exit: reference exit 0, target exit 1".into());
        let p = prompt(&req);
        assert!(p.contains("specs/linux/x86_64/brk.yaml") && p.contains("specs/linux/x86_64/brk.tests"));
        assert!(p.contains("revision abc123") && p.contains("must NOT run"));
        assert!(p.ends_with("exit: reference exit 0, target exit 1\n"));
        let s = prompt(&request(Role::Spec, &e, Path::new("."), "true"));
        assert!(s.contains("SPEC AGENT") && s.contains("OUTSIDE the repository"));
        assert!(!s.contains("continues your previous one"));
        // A failed spec check: the spec agent continues its own work.
        let mut req = request(Role::Spec, &e, Path::new("."), "true");
        req.feedback = Some("`cargo xtask ci` failed".into());
        let s = prompt(&req);
        assert!(s.contains("continues your previous one") && s.contains("--replace") && !s.contains("diff runner"));
        assert!(s.ends_with("`cargo xtask ci` failed\n"));
    }

    #[test]
    fn limits_are_detected() {
        let p = limit_patterns();
        assert!(hit_limit("Claude AI usage limit reached|1760000000", &p));
        assert!(hit_limit("Error: 429 Too Many Requests", &p));
        assert!(!hit_limit("all tests passed", &p));
    }

    #[test]
    fn runs_the_command_with_the_role_environment() {
        let base = std::env::temp_dir().join(format!("poly-roles-test-{}", std::process::id()));
        let work = base.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let e = entry();
        let cmd = "echo role=$POLY_ROLE entry=$POLY_ENTRY heldout=${POLY_HELDOUT_DIR:-none}; head -c 23 {prompt_file}";
        let out = run(&crate::repo_root(), &base.join("logs"), &request(Role::Implementer, &e, &work, cmd)).unwrap();
        assert!(out.success && !out.limit_reached);
        let log = std::fs::read_to_string(&out.log).unwrap();
        assert!(log.starts_with("role=implementer entry=linux/syscall/x86_64/brk heldout=none\n"), "{log}");
        assert!(log.contains("You are the IMPLEMENTER"));
        let limited =
            run(&crate::repo_root(), &base.join("logs"), &request(Role::Spec, &e, &work, "echo 'usage limit reached'"))
                .unwrap();
        assert!(limited.limit_reached && !limited.success);
        let _ = std::fs::remove_dir_all(&base);
    }
}
