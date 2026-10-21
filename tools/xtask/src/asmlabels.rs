//! `cargo xtask ci` check: every named label in the kernel's `global_asm!`
//! blocks must be unique.
//!
//! All `global_asm!` of a crate are assembled into whichever codegen unit
//! the compiler picks, so two files that each define `fail:` build on one
//! machine and fail with "symbol 'fail' is already defined" on another.
//! Numeric local labels (`1:`, `jnz 1f`) may repeat and are not checked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn check(kernel_src: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect(kernel_src, &mut files)?;
    let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        if !text.contains("global_asm!") {
            continue;
        }
        let shown = file.strip_prefix(kernel_src).unwrap_or(file).display().to_string();
        for (line, label) in labels(&text) {
            seen.entry(label).or_default().push(format!("{shown}:{line}"));
        }
    }
    let clashes: Vec<String> = seen
        .into_iter()
        .filter(|(_, places)| places.len() > 1)
        .map(|(label, places)| format!("  `{label}:` at {}", places.join(", ")))
        .collect();
    if clashes.is_empty() {
        return Ok(());
    }
    Err(format!(
        "global_asm! labels defined more than once (prefix them with the module name, \
         or use numeric local labels):\n{}",
        clashes.join("\n")
    ))
}

/// The named labels a file defines, one per line that holds only `name:`,
/// with their line numbers.
fn labels(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let name = line.trim().strip_suffix(':')?;
            let named = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '.');
            let label = name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$');
            // `.L` labels are assembler-local but still clash within one
            // codegen unit, so they count too.
            (named && label && !name.is_empty()).then(|| (i + 1, name.to_string()))
        })
        .collect()
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn finds_named_labels_only() {
        let text = "global_asm!(\"\n    .global start\nstart:\n1:\n    jnz fail\nfail:\n    mov eax, {x}\n\");\n";
        let found: Vec<_> = labels(text).into_iter().map(|(_, l)| l).collect();
        assert_eq!(found, ["start", "fail"]);
    }

    #[test]
    fn rust_code_is_not_a_label() {
        // Match arms and struct fields end in other characters.
        assert!(labels("    Some(x) => {\n    field: u32,\n    'outer: loop {\n").is_empty());
    }
}
