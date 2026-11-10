//! `cargo xtask ci` check: no tracked file holds a leftover merge conflict
//! marker (a line starting with `<<<<<<< ` or `>>>>>>> `). A conflict
//! resolved by hand in a file nothing compiles, such as the docs, passes
//! every other check with its markers still in it.

use std::path::Path;
use std::process::Command;

/// Check every tracked file, or only the files `only` names (`ci lint
/// --quick`: the files changed against `origin/main`; one that is gone is
/// skipped).
pub fn check(root: &Path, only: Option<&[String]>) -> Result<(), String> {
    let names: Vec<String> = match only {
        Some(files) => files.to_vec(),
        None => {
            let out = Command::new("git")
                .current_dir(root)
                .args(["ls-files", "-z"])
                .output()
                .map_err(|e| format!("git ls-files: {e}"))?;
            if !out.status.success() {
                return Err("git ls-files failed".into());
            }
            out.stdout
                .split(|b| *b == 0)
                .filter(|n| !n.is_empty())
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .collect()
        }
    };
    let mut found = Vec::new();
    for name in &names {
        // Binary files and generated images may hold anything.
        let Ok(text) = std::fs::read_to_string(root.join(name)) else { continue };
        for (line, at) in markers(&text) {
            found.push(format!("  {name}:{at}: {line}"));
        }
    }
    if found.is_empty() {
        return Ok(());
    }
    Err(format!("merge conflict markers left in tracked files:\n{}", found.join("\n")))
}

/// The marker lines of `text` with their line numbers.
fn markers(text: &str) -> Vec<(&str, usize)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> "))
        .map(|(i, l)| (l, i + 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::markers;

    #[test]
    fn finds_both_ends_of_a_conflict() {
        let text = "a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> main\nd\n";
        assert_eq!(markers(text), [("<<<<<<< HEAD", 2), (">>>>>>> main", 6)]);
    }

    #[test]
    fn setext_headings_and_quotes_are_no_conflict() {
        // `=======` underlines a Markdown heading; `>>>` quotes nest.
        assert!(markers("Title\n=======\n> > > quoted\n>>>>>>>no space\n").is_empty());
    }
}
