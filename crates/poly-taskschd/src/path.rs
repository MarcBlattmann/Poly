//! Task and folder paths: `\` separates the components, a leading `\` is the root (or, asked of a
//! folder object, ignored), the lookup ignores case. What each form gives (sheets
//! `ITaskService.GetFolder`, `ITaskFolder.CreateFolder`, `ITaskFolder.GetTask`,
//! `ITaskFolder.DeleteTask`, `ITaskFolder.RegisterTaskDefinition`):
//!
//! | Form | folder / task lookups | registration |
//! |---|---|---|
//! | trailing or doubled `\` | `0x8007007B` | `0x80070057` |
//! | `:` or `/` in a component | `0x8007007B` | `0x80070057` |
//! | `..`, a leading space | `0x8007007B` | `0x8007007B` |
//! | `*` `?` `<` `>` `|` | not found (`0x80070002`) | `0x8007007B` |

use crate::hr::{E_INVALIDARG, Hresult, INVALID_NAME};
use alloc::string::String;
use alloc::vec::Vec;

/// The longest component a folder or task name may have.
pub const MAX_NAME: usize = 255;

/// The components of `p` for a lookup (`GetFolder`, `GetTask`, `DeleteTask`, `DeleteFolder`), or
/// the error its form gives. `""` and `"\"` are no components (the folder itself).
pub fn components(p: &str) -> Result<Vec<String>, Hresult> {
    if p.is_empty() || p == "\\" {
        return Ok(Vec::new());
    }
    let rest = p.strip_prefix('\\').unwrap_or(p);
    if rest.ends_with('\\') {
        return Err(INVALID_NAME);
    }
    let mut out = Vec::new();
    for c in rest.split('\\') {
        if c.is_empty() || c == ".." || c.starts_with(' ') || c.contains([':', '/']) {
            return Err(INVALID_NAME);
        }
        out.push(String::from(c));
    }
    Ok(out)
}

/// The components of a path given to `CreateFolder`: as [`components`], and a name longer than
/// [`MAX_NAME`] is `E_INVALIDARG`.
pub fn folder_to_create(p: &str) -> Result<Vec<String>, Hresult> {
    let c = components(p)?;
    if c.iter().any(|n| n.chars().count() > MAX_NAME) {
        return Err(E_INVALIDARG);
    }
    Ok(c)
}

/// The components of a path given to `RegisterTaskDefinition` / `RegisterTask` (not empty; the
/// caller handles the empty and NULL path).
pub fn task_to_register(p: &str) -> Result<Vec<String>, Hresult> {
    if p == "\\" {
        return Err(E_INVALIDARG);
    }
    let rest = p.strip_prefix('\\').unwrap_or(p);
    if rest.is_empty() || rest.ends_with('\\') {
        return Err(E_INVALIDARG);
    }
    let mut out = Vec::new();
    for c in rest.split('\\') {
        if c.is_empty() || c.contains([':', '/']) || c.chars().count() > MAX_NAME {
            return Err(E_INVALIDARG);
        }
        if c == ".." || c.starts_with(' ') || c.contains(['*', '?', '<', '>', '|']) {
            return Err(INVALID_NAME);
        }
        out.push(String::from(c));
    }
    Ok(out)
}

/// Whether two names are the same name (case does not count).
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.chars().zip(b.chars()).all(|(x, y)| x.to_lowercase().eq(y.to_lowercase()))
}

/// The key two names are sorted by: alphabetical without regard to case.
pub fn sort_key(s: &str) -> String {
    s.to_lowercase()
}

/// `\a\b` for the components `a`, `b`; `\` for none.
pub fn join(c: &[String]) -> String {
    if c.is_empty() {
        return String::from("\\");
    }
    let mut s = String::new();
    for n in c {
        s.push('\\');
        s.push_str(n);
    }
    s
}

/// The path text of an object below a folder object whose path text is `folder`: `folder\name`
/// (`\name` below the root).
pub fn child_text(folder: &str, name: &str) -> String {
    let name = name.strip_prefix('\\').unwrap_or(name);
    let mut s = String::from(folder.trim_end_matches('\\'));
    s.push('\\');
    s.push_str(name);
    s
}

/// The last component of a path text (`\` for the root).
pub fn last(p: &str) -> String {
    let t = p.trim_end_matches('\\');
    if t.is_empty() {
        return String::from("\\");
    }
    String::from(t.rsplit('\\').next().unwrap_or(t))
}
