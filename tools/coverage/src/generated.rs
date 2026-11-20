//! What is generated, and only on `main` (ADR 0213): the README's progress
//! badges, the detailed progress page (`docs/progress/README.md`),
//! `docs/progress/*.svg` and the ADR index in `docs/adr/README.md`.
//!
//! Every pull request that touched `coverage/` or the roadmap used to
//! rewrite these, so every merge made the next pull request conflict in
//! them. Now a workflow on `main` regenerates them (`cargo xtask coverage
//! readme`), and a pull request must leave them alone: [`changed`] finds
//! what a branch changed in them, compared with the commit it branched from.

use crate::readme::{ASSET_DIR, BADGES_BEGIN, BADGES_END, BEGIN, END, PHASES_BEGIN, PHASES_END, PROGRESS_DOC};
use std::fmt::Write as _;
use std::path::Path;

/// The ADR index's markers in `docs/adr/README.md`.
pub const ADR_BEGIN: &str = "<!-- adr-index:begin -->";
pub const ADR_END: &str = "<!-- adr-index:end -->";

/// The ADR index's file, relative to the repo root.
pub const ADR_INDEX: &str = "docs/adr/README.md";

/// A generated part of a Markdown file: the text from `begin` to `end`.
pub struct Region {
    pub file: &'static str,
    pub name: &'static str,
    pub begin: &'static str,
    pub end: &'static str,
}

/// The generated parts of the README, the progress page and the ADR index.
pub const REGIONS: [Region; 4] = [
    Region { file: "README.md", name: "the README's progress badges", begin: BADGES_BEGIN, end: BADGES_END },
    Region { file: PROGRESS_DOC, name: "the progress section", begin: BEGIN, end: END },
    Region { file: PROGRESS_DOC, name: "the phase table", begin: PHASES_BEGIN, end: PHASES_END },
    Region { file: ADR_INDEX, name: "the ADR index", begin: ADR_BEGIN, end: ADR_END },
];

/// The text from `begin` to `end` (both included), if both are there in order.
pub fn region<'a>(text: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let start = text.find(begin)?;
    let stop = text.find(end)? + end.len();
    (start < stop).then(|| &text[start..stop])
}

/// What of the generated files differs between `base` and the branch, one
/// line each, in words. Each `(file, text)` is a Markdown file as it is on
/// the base (`None`: not there) and now. A part that the base does not have
/// yet (a region introduced by the very change that adds it) is not
/// compared. `progress_files` are the files under `docs/progress/` that
/// differ.
pub fn changed(
    base: &dyn Fn(&str) -> Option<String>,
    now: &dyn Fn(&str) -> Option<String>,
    progress_files: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    for r in &REGIONS {
        let (Some(base_text), Some(now_text)) = (base(r.file), now(r.file)) else { continue };
        let Some(before) = region(&base_text, r.begin, r.end) else { continue };
        if region(&now_text, r.begin, r.end) != Some(before) {
            out.push(format!("{} in {}", r.name, r.file));
        }
    }
    out.extend(progress_files.iter().cloned());
    out
}

/// The message of a failed check: what changed, and why it must not.
pub fn explain(changed: &[String]) -> String {
    format!(
        "this change rewrites generated files:\n  {}\nThey are regenerated on main after every merge \
         (`progress.yml`, ADR 0213), so a pull request must not change them: restore them from main \
         (`git checkout origin/main -- docs/progress`, and take main's text of the generated sections \
         of README.md, docs/progress/README.md and docs/adr/README.md; the rest of those files stays editable). \
         `cargo xtask coverage readme` still previews them locally; do not commit its output.",
        changed.join("\n  ")
    )
}

/// The title of an ADR: its first line, `# 0213: The title`.
fn title(text: &str) -> Option<&str> {
    let first = text.lines().next()?.strip_prefix("# ")?;
    let (number, title) = first.split_once(": ")?;
    (number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit())).then_some(title.trim())
}

/// The status of an ADR: its `**Status:**` line up to the first `. `, `;`, `,`,
/// `:` or ` (` ("Accepted. Builds on ..." and "Accepted (owner decision)" are
/// "Accepted").
fn status(text: &str) -> &str {
    let Some(line) = text.lines().find_map(|l| l.strip_prefix("**Status:**")) else { return "?" };
    let line = line.trim();
    let cut = [line.find(". "), line.find([';', ',', ':']), line.find(" ("), line.strip_suffix('.').map(str::len)]
        .into_iter()
        .flatten()
        .min();
    line[..cut.unwrap_or(line.len())].trim()
}

/// The ADR index's section: a table row for every `NNNN-*.md` in `dir`, in
/// number order, from the files' titles and statuses.
pub fn adr_index(dir: &Path) -> Result<String, String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.len() > 5 && n.ends_with(".md") && n.as_bytes()[..4].iter().all(u8::is_ascii_digit))
        .collect();
    names.sort();
    let mut out = format!(
        "{ADR_BEGIN}\n<!-- Generated by `cargo xtask coverage readme` from the ADR files' titles and statuses \
         (ADR 0213). Do not edit. -->\n\n| # | Decision | Status |\n|---|---|---|\n"
    );
    for name in &names {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let title =
            title(&text).ok_or_else(|| format!("{}: the first line must be `# NNNN: title`", path.display()))?;
        let _ = writeln!(
            out,
            "| [{}]({name}) | {} | {} |",
            &name[..4],
            title.replace('|', "\\|"),
            status(&text).replace('|', "\\|")
        );
    }
    out.push_str(ADR_END);
    Ok(out)
}

/// The generated graphics of `docs/progress/`, for the message.
/// (`docs/progress/README.md` is checked region by region via [`REGIONS`].)
pub fn is_progress_file(path: &str) -> bool {
    path.starts_with(&format!("{ASSET_DIR}/")) && path.ends_with(".svg")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn readme(badges: &str, mine: &str) -> String {
        format!("top {mine}\n{BADGES_BEGIN}\n{badges}\n{BADGES_END}\nend\n")
    }

    fn progress_doc(coverage: &str, phases: &str, mine: &str) -> String {
        format!("top {mine}\n{BEGIN}\n{coverage}\n{END}\nmid\n{PHASES_BEGIN}\n{phases}\n{PHASES_END}\nend\n")
    }

    fn lookup(files: &[(&str, String)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: BTreeMap<String, String> = files.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        move |f: &str| map.get(f).cloned()
    }

    #[test]
    fn only_the_generated_parts_count() {
        let base = lookup(&[("README.md", readme("b", "x")), (PROGRESS_DOC, progress_doc("a", "p", "x"))]);
        // The rest of both files is editable.
        let edited = lookup(&[("README.md", readme("b", "edited")), (PROGRESS_DOC, progress_doc("a", "p", "edited"))]);
        assert!(changed(&base, &edited, &[]).is_empty());
        // None of the generated sections may change.
        let c = changed(
            &base,
            &lookup(&[("README.md", readme("b2", "x")), (PROGRESS_DOC, progress_doc("a", "p", "x"))]),
            &[],
        );
        assert_eq!(c, ["the README's progress badges in README.md"]);
        let c = changed(
            &base,
            &lookup(&[("README.md", readme("b", "x")), (PROGRESS_DOC, progress_doc("a2", "p", "x"))]),
            &[],
        );
        assert_eq!(c, [format!("the progress section in {PROGRESS_DOC}")]);
        let c = changed(
            &base,
            &lookup(&[("README.md", readme("b", "x")), (PROGRESS_DOC, progress_doc("a", "q", "x"))]),
            &[],
        );
        assert_eq!(c, [format!("the phase table in {PROGRESS_DOC}")]);
        // Nor the graphics.
        assert!(is_progress_file("docs/progress/badge-linux.svg"));
        assert!(!is_progress_file(PROGRESS_DOC));
        let c = changed(&base, &base, &["docs/progress/badge-linux.svg".to_string()]);
        assert_eq!(c, ["docs/progress/badge-linux.svg"]);
        assert!(explain(&c).contains("regenerated on main"));
    }

    #[test]
    fn a_region_the_base_lacks_is_not_compared() {
        let with = format!("intro\n{ADR_BEGIN}\nrows\n{ADR_END}\n");
        let base = lookup(&[("docs/adr/README.md", "intro\nold table\n".to_string())]);
        assert!(changed(&base, &lookup(&[("docs/adr/README.md", with.clone())]), &[]).is_empty());
        let base = lookup(&[("docs/adr/README.md", with)]);
        let now = format!("intro\n{ADR_BEGIN}\nmore rows\n{ADR_END}\n");
        assert_eq!(
            changed(&base, &lookup(&[("docs/adr/README.md", now)]), &[]),
            ["the ADR index in docs/adr/README.md"]
        );
    }

    #[test]
    fn titles_and_statuses() {
        assert_eq!(title("# 0210: A | B\n"), Some("A | B"));
        assert_eq!(title("# ADR 1: x"), None);
        assert_eq!(status("# t\n\n**Status:** Accepted. Builds on [0089](x.md).\n"), "Accepted");
        assert_eq!(status("**Status:** Accepted; storage layout superseded by [0006](a.md)"), "Accepted");
        assert_eq!(status("**Status:** Superseded by [0050](0050-x.md).\n"), "Superseded by [0050](0050-x.md)");
        assert_eq!(status("**Status:** Accepted\n"), "Accepted");
        assert_eq!(status("**Status:** Accepted (owner decision, 2026-10-20)\n"), "Accepted");
        assert_eq!(status("no status"), "?");
    }

    #[test]
    fn index_rows_come_from_the_files_in_number_order() {
        let dir = std::env::temp_dir().join(format!("poly-adr-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0002-b.md"), "# 0002: B | pipe\n\n**Status:** Accepted. More\n").unwrap();
        std::fs::write(dir.join("0001-a.md"), "# 0001: A\n\n**Status:** Accepted\n").unwrap();
        std::fs::write(dir.join("README.md"), "not an ADR").unwrap();
        let index = adr_index(&dir).unwrap();
        let rows: Vec<&str> = index.lines().filter(|l| l.starts_with("| [")).collect();
        assert_eq!(rows, ["| [0001](0001-a.md) | A | Accepted |", "| [0002](0002-b.md) | B \\| pipe | Accepted |"]);
        assert!(index.starts_with(ADR_BEGIN) && index.ends_with(ADR_END));
        std::fs::write(dir.join("0003-c.md"), "no title").unwrap();
        assert!(adr_index(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
