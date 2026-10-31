//! Wildcard expansion of `argv` (Learn: "Expanding Wildcard Arguments":
//! a program linked with `setargv.obj` / `wsetargv.obj` gets each argument
//! with `?` or `*` replaced by the names of the files it matches; the
//! UCRT's start-up code asks for it with `_crt_argv_expanded_arguments`,
//! `corecrt_startup.h`, and `msvcrt.dll`'s `__getmainargs` with its
//! `DoWildCard` argument).
//!
//! The matching itself is the file system's (`FindFirstFileW` on the
//! argument, so wildcards count in the last path component only). What
//! Learn leaves open, Poly's choices (to be confirmed by the spec agent):
//!
//! - `argv[0]` is never expanded; neither is an argument a double quote
//!   delimited part of (`"*.c"` stays `*.c`).
//! - Each name found keeps the argument's directory part (everything up to
//!   its last `\`, `/` or `:`), so `src\*.c` gives `src\a.c`.
//! - `.` and `..` are left out; the names come in the order the directory
//!   lists them.
//! - An argument that matches nothing stays as it was.

use crate::unit::Unit;
use alloc::vec::Vec;

/// Whether `arg` has a `?` or `*`.
pub fn has_wildcards<C: Unit>(arg: &[C]) -> bool {
    arg.iter().any(|c| matches!(c.as_ascii(), Some(b'*' | b'?')))
}

/// The length of `arg`'s directory part, with its last separator.
pub fn dir_len<C: Unit>(arg: &[C]) -> usize {
    arg.iter().rposition(|c| matches!(c.as_ascii(), Some(b'\\' | b'/' | b':'))).map_or(0, |i| i + 1)
}

fn is_dot_entry<C: Unit>(name: &[C]) -> bool {
    name.iter().all(|c| c.as_ascii() == Some(b'.')) && (1..=2).contains(&name.len())
}

/// Expand `args` (from [`crate::args::parse_marked`]): `find(pattern)` lists
/// the names (last components only) a pattern matches.
pub fn expand<C: Unit>(args: Vec<(Vec<C>, bool)>, mut find: impl FnMut(&[C]) -> Vec<Vec<C>>) -> Vec<Vec<C>> {
    let mut out = Vec::with_capacity(args.len());
    for (i, (arg, quoted)) in args.into_iter().enumerate() {
        if i == 0 || quoted || !has_wildcards(&arg) {
            out.push(arg);
            continue;
        }
        let dir = &arg[..dir_len(&arg)];
        let names: Vec<Vec<C>> = find(&arg).into_iter().filter(|n| !is_dot_entry(n)).collect();
        if names.is_empty() {
            out.push(arg);
            continue;
        }
        for n in names {
            let mut full = dir.to_vec();
            full.extend_from_slice(&n);
            out.push(full);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::parse_marked;
    use alloc::string::String;

    fn names(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    fn run(line: &str) -> Vec<String> {
        let found = |pattern: &[u8]| -> Vec<Vec<u8>> {
            match pattern {
                b"*.c" => names(&["a.c", "b.c"]),
                b"src\\?.h" | b"C:x*" => names(&["x.h"]),
                b"*" => names(&[".", "..", "f"]),
                _ => Vec::new(),
            }
        };
        expand(parse_marked(line.as_bytes()), found).into_iter().map(|a| String::from_utf8(a).unwrap()).collect()
    }

    #[test]
    fn patterns_become_names() {
        assert_eq!(run("prog *.c -o out"), ["prog", "a.c", "b.c", "-o", "out"]);
        assert_eq!(run(r"prog src\?.h C:x*"), ["prog", r"src\x.h", "C:x.h"]);
        assert_eq!(run("prog *"), ["prog", "f"]);
    }

    #[test]
    fn what_stays() {
        // argv[0], quoted arguments, no match, no wildcard.
        assert_eq!(run("*.c"), ["*.c"]);
        assert_eq!(run(r#"prog "*.c" x"*.c" *.zz plain"#), ["prog", "*.c", "x*.c", "*.zz", "plain"]);
        assert_eq!(run(r"prog a\*.c"), ["prog", r"a\*.c"]);
    }

    #[test]
    fn helpers() {
        assert!(has_wildcards(b"a?") && !has_wildcards(b"abc"));
        assert_eq!(dir_len(b"a/b\\c*"), 4);
        assert_eq!(dir_len(b"D:*.x"), 2);
        assert_eq!(dir_len(b"*.x"), 0);
        let w: Vec<u16> = "d\\*".encode_utf16().collect();
        assert!(has_wildcards(&w) && dir_len(&w) == 2);
    }
}
