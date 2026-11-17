//! How many bytes of arguments a 32-bit `stdcall` function pops, at compile time (ADR 0186,
//! ADR 0222): the lookup in `x86_signatures.txt` that the fail-only exports of Poly's own i686
//! modules (`poly_win32_rt::fail_exports!`) need, as a `const fn` without dependencies, so the
//! Win32 runtime includes this very file (`#[path]`) and `poly-stubs` tests it on the host.
//!
//! A line of the table is `dll!name slots [cdecl]`; a name `ord_N` of a module is the table's
//! `dll!#N`. The answer is the bytes a `ret n` pops: `4 * slots`, 0 for `cdecl`, or -1 when the
//! table does not know the function (its stub must not return then).

/// The bytes the 32-bit stub of `name` in `dll` pops, from `table`; -1 if unknown.
pub const fn stack_bytes(table: &str, dll: &str, name: &str) -> i32 {
    let t = table.as_bytes();
    let mut i = 0;
    while i < t.len() {
        // One line: `start .. end`.
        let start = i;
        while i < t.len() && t[i] != b'\n' {
            i += 1;
        }
        let end = i;
        i += 1;
        let r = line_bytes(t, start, end, dll.as_bytes(), name.as_bytes());
        if r != -2 {
            return r;
        }
    }
    -1
}

/// The answer of line `t[start..end]` for `dll!name`, or -2 when it is about another function.
const fn line_bytes(t: &[u8], start: usize, end: usize, dll: &[u8], name: &[u8]) -> i32 {
    let mut p = skip_space(t, start, end);
    if p >= end || t[p] == b'#' {
        return -2;
    }
    // The DLL, case ignored.
    let mut k = 0;
    while k < dll.len() {
        if p >= end || t[p].to_ascii_lowercase() != dll[k].to_ascii_lowercase() {
            return -2;
        }
        p += 1;
        k += 1;
    }
    if p >= end || t[p] != b'!' {
        return -2;
    }
    p += 1;
    // The name, or `#N` for `ord_N`.
    let (name, ordinal) = if starts_with(name, b"ord_") { (split_at(name, 4), true) } else { (name, false) };
    if ordinal {
        if p >= end || t[p] != b'#' {
            return -2;
        }
        p += 1;
    }
    let mut k = 0;
    while k < name.len() {
        if p >= end || t[p] != name[k] {
            return -2;
        }
        p += 1;
        k += 1;
    }
    if p >= end || !is_space(t[p]) {
        return -2;
    }
    p = skip_space(t, p, end);
    let mut slots = 0i32;
    let mut digits = 0;
    while p < end && t[p].is_ascii_digit() {
        slots = slots * 10 + (t[p] - b'0') as i32;
        p += 1;
        digits += 1;
    }
    if digits == 0 {
        return -1;
    }
    p = skip_space(t, p, end);
    if p + 5 <= end && t[p] == b'c' && t[p + 1] == b'd' && t[p + 2] == b'e' && t[p + 3] == b'c' && t[p + 4] == b'l' {
        return 0;
    }
    slots * 4
}

const fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t' || c == b'\r'
}

const fn skip_space(t: &[u8], mut p: usize, end: usize) -> usize {
    while p < end && is_space(t[p]) {
        p += 1;
    }
    p
}

const fn starts_with(s: &[u8], prefix: &[u8]) -> bool {
    if s.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if s[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn split_at(s: &[u8], at: usize) -> &[u8] {
    s.split_at(at).1
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "# a comment\n\nkernel32!GetDriveTypeW 1\r\nshell32!ShellExecuteExW 1\nuser32!wsprintfA 2 cdecl\ncomctl32!#17 0\nshell32!ShellExecuteW 6\n";

    #[test]
    fn known_names_give_their_bytes() {
        assert_eq!(stack_bytes(TABLE, "kernel32", "GetDriveTypeW"), 4);
        assert_eq!(stack_bytes(TABLE, "shell32", "ShellExecuteW"), 24);
        assert_eq!(stack_bytes(TABLE, "SHELL32", "ShellExecuteExW"), 4);
        // cdecl: the caller pops.
        assert_eq!(stack_bytes(TABLE, "user32", "wsprintfA"), 0);
        // An ordinal export.
        assert_eq!(stack_bytes(TABLE, "comctl32", "ord_17"), 0);
    }

    #[test]
    fn unknown_names_and_prefixes_are_not_matched() {
        assert_eq!(stack_bytes(TABLE, "kernel32", "GetDriveType"), -1);
        assert_eq!(stack_bytes(TABLE, "kernel32", "GetDriveTypeWX"), -1);
        assert_eq!(stack_bytes(TABLE, "shell32", "ShellExecuteA"), -1);
        assert_eq!(stack_bytes(TABLE, "comctl32", "ord_1"), -1);
        assert_eq!(stack_bytes("", "a", "b"), -1);
    }

    #[test]
    fn it_works_in_a_constant_and_agrees_with_the_table_reader() {
        const B: i32 = stack_bytes(include_str!("x86_signatures.txt"), "advapi32", "RegLoadKeyW");
        assert_eq!(B, 12);
        for line in include_str!("x86_signatures.txt").lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let (key, rest) = l.split_once(char::is_whitespace).unwrap();
            let (dll, name) = key.split_once('!').unwrap();
            let name = match name.strip_prefix('#') {
                Some(n) => std::format!("ord_{n}"),
                None => name.into(),
            };
            let sig = crate::x86_signature(dll, key.split_once('!').unwrap().1).unwrap();
            let want = if sig.cdecl { 0 } else { 4 * i32::from(sig.slots) };
            assert_eq!(stack_bytes(include_str!("x86_signatures.txt"), dll, &name), want, "{l} ({rest})");
        }
    }

    extern crate std;
}
