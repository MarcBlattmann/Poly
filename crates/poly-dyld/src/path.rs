//! Install names to file paths: `@executable_path`, `@loader_path` and
//! `@rpath`, as the dyld(1) manual page describes them.
//!
//! - `@executable_path/` is the directory of the main program.
//! - `@loader_path/` is the directory of the image whose load command
//!   names the library.
//! - `@rpath/` is tried with each run path (`LC_RPATH`) of the image that
//!   names the library, then of the image that loaded that one, and so on up
//!   to the main program. A run path may itself start with
//!   `@executable_path/` or `@loader_path/` (the loader being the image
//!   that has the `LC_RPATH`).
//!
//! Paths are joined and cleaned up lexically (`.` and `..` components,
//! repeated slashes): `/bin/../usr/lib/x.dylib` becomes `/usr/lib/x.dylib`.

use alloc::string::String;
use alloc::vec::Vec;

const EXECUTABLE_PATH: &str = "@executable_path";
const LOADER_PATH: &str = "@loader_path";
const RPATH: &str = "@rpath";

/// The directory part of `path`: everything before the last `/` (`.` if
/// there is none, `/` for a file in the root).
pub fn dirname(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => ".",
    }
}

/// `path` with `.` and empty components removed and `..` applied. An
/// absolute path stays absolute (`..` above the root stays at the root); a
/// relative one keeps leading `..`.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&p) if p != ".." => {
                    parts.pop();
                }
                _ if absolute => {}
                _ => parts.push(".."),
            },
            c => parts.push(c),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('.');
    }
    out
}

/// `rest` (after an `@...` prefix, starting with `/` or empty) appended to
/// the directory `dir`.
fn join(dir: &str, rest: &str) -> String {
    let mut s = String::from(dir);
    s.push('/');
    s.push_str(rest.trim_start_matches('/'));
    normalize(&s)
}

/// `name` with a leading `@executable_path` or `@loader_path` replaced;
/// `None` if it starts with another `@` word (such as `@rpath`), which
/// needs a search.
pub fn expand(name: &str, executable: &str, loader: &str) -> Option<String> {
    if let Some(rest) = strip_word(name, EXECUTABLE_PATH) {
        return Some(join(dirname(executable), rest));
    }
    if let Some(rest) = strip_word(name, LOADER_PATH) {
        return Some(join(dirname(loader), rest));
    }
    if name.starts_with('@') {
        return None;
    }
    Some(normalize(name))
}

/// `name` without the `@` word `word` if it starts with it, followed by
/// `/` or the end.
fn strip_word<'a>(name: &'a str, word: &str) -> Option<&'a str> {
    let rest = name.strip_prefix(word)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// One image on the way from the library being looked up to the main
/// program: its path and its run paths (`LC_RPATH`, as written).
#[derive(Clone, Copy, Debug)]
pub struct Loader<'a> {
    pub path: &'a str,
    pub rpaths: &'a [String],
}

/// The paths to try for the install name `name`, in order: one for a plain
/// path or an `@executable_path` / `@loader_path` one, one per run path of
/// each image in `chain` for an `@rpath` one. `chain[0]` is the image whose
/// load command names the library, the last one the main program.
pub fn candidates(name: &str, executable: &str, chain: &[Loader<'_>]) -> Vec<String> {
    let loader = chain.first().map_or(executable, |l| l.path);
    let mut out = Vec::new();
    if let Some(rest) = strip_word(name, RPATH) {
        for l in chain {
            for rp in l.rpaths {
                if let Some(dir) = expand(rp, executable, l.path) {
                    let p = join(&dir, rest);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    } else if let Some(p) = expand(name, executable, loader) {
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("/bin/../usr/lib//x.dylib"), "/usr/lib/x.dylib");
        assert_eq!(normalize("/../a/./b/"), "/a/b");
        assert_eq!(normalize("a/../../b"), "../b");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize(""), ".");
    }

    #[test]
    fn dirnames() {
        assert_eq!(dirname("/bin/hello"), "/bin");
        assert_eq!(dirname("/hello"), "/");
        assert_eq!(dirname("hello"), ".");
    }

    #[test]
    fn expands_executable_and_loader_path() {
        let exe = "/Applications/A.app/Contents/MacOS/A";
        let loader = "/Applications/A.app/Contents/Frameworks/F.framework/F";
        assert_eq!(
            expand("@executable_path/../Frameworks/x.dylib", exe, loader).unwrap(),
            "/Applications/A.app/Contents/Frameworks/x.dylib"
        );
        assert_eq!(
            expand("@loader_path/lib/y.dylib", exe, loader).unwrap(),
            "/Applications/A.app/Contents/Frameworks/F.framework/lib/y.dylib"
        );
        assert_eq!(expand("/usr/lib/libSystem.B.dylib", exe, loader).unwrap(), "/usr/lib/libSystem.B.dylib");
        assert_eq!(expand("@rpath/z.dylib", exe, loader), None);
        // `@executable_pathX` is not the word.
        assert_eq!(expand("@executable_pathX/a", exe, loader), None);
    }

    #[test]
    fn rpath_searches_the_chain_in_order() {
        let lib_rp = vec!["@loader_path/../lib".to_string()];
        let exe_rp = vec!["@executable_path/../Frameworks".to_string(), "/opt/lib".to_string()];
        let chain = [
            Loader { path: "/app/Frameworks/libA.dylib", rpaths: &lib_rp },
            Loader { path: "/app/MacOS/prog", rpaths: &exe_rp },
        ];
        assert_eq!(
            candidates("@rpath/libB.dylib", "/app/MacOS/prog", &chain),
            ["/app/lib/libB.dylib", "/app/Frameworks/libB.dylib", "/opt/lib/libB.dylib"]
        );
        // No run paths: nothing to try.
        assert!(candidates("@rpath/libB.dylib", "/p", &[Loader { path: "/p", rpaths: &[] }]).is_empty());
        assert_eq!(candidates("@loader_path/x.dylib", "/p", &chain), ["/app/Frameworks/x.dylib"]);
    }
}
