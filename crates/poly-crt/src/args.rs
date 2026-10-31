//! `argv` from the command line, as Learn's "Parsing C command-line
//! arguments" documents it, and `_environ` from the environment block.
//!
//! - Arguments are separated by spaces or tabs.
//! - `argv[0]`, the program name, ends at the first space or tab outside
//!   double quotes; quotes are removed and nothing else is special.
//! - Elsewhere a quoted string is part of an argument whatever it holds;
//!   inside quotes, `""` is one literal quote; an unclosed quote runs to the
//!   end.
//! - `2n` backslashes before a quote give `n` backslashes and the quote
//!   delimits; `2n + 1` give `n` backslashes and a literal quote;
//!   backslashes not before a quote are literal.
//!
//! Poly's choices (to be confirmed by the spec agent): a command line that
//! starts with a space has an empty `argv[0]`; `_environ` leaves out the
//! block's `=X:` drive-directory entries (every entry starting with `=`).

use crate::unit::Unit;
use alloc::vec::Vec;

fn is(c: u32, b: u8) -> bool {
    c == u32::from(b)
}

/// Split a command line (without its NUL) into arguments.
pub fn parse<C: Unit>(line: &[C]) -> Vec<Vec<C>> {
    parse_marked(line).into_iter().map(|(a, _)| a).collect()
}

/// [`parse`], each argument with whether a double quote delimited part
/// of it (wildcard expansion leaves such arguments as they are,
/// [`crate::wildcard`]).
pub fn parse_marked<C: Unit>(line: &[C]) -> Vec<(Vec<C>, bool)> {
    let at = |i: usize| line.get(i).map_or(0, |c| c.to_u32());
    let blank = |c: u32| is(c, b' ') || is(c, b'\t');
    let n = line.len();
    let mut args = Vec::new();
    // The program name.
    let mut i = 0;
    let mut name = Vec::new();
    let mut quoted = false;
    let mut had_quote = false;
    while i < n {
        let c = at(i);
        if is(c, b'"') {
            quoted = !quoted;
            had_quote = true;
        } else if blank(c) && !quoted {
            break;
        } else {
            name.push(line[i]);
        }
        i += 1;
    }
    args.push((name, had_quote));
    loop {
        while i < n && blank(at(i)) {
            i += 1;
        }
        if i >= n {
            break;
        }
        let mut arg = Vec::new();
        let mut quoted = false;
        let mut had_quote = false;
        while i < n {
            let c = at(i);
            if is(c, b'\\') {
                let start = i;
                while i < n && is(at(i), b'\\') {
                    i += 1;
                }
                let count = i - start;
                if i < n && is(at(i), b'"') {
                    arg.extend(core::iter::repeat_n(C::ascii(b'\\'), count / 2));
                    if count % 2 == 1 {
                        arg.push(C::ascii(b'"'));
                        i += 1;
                    }
                } else {
                    arg.extend(core::iter::repeat_n(C::ascii(b'\\'), count));
                }
                continue;
            }
            if is(c, b'"') {
                if quoted && is(at(i + 1), b'"') && i + 1 < n {
                    arg.push(C::ascii(b'"'));
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                had_quote = true;
                i += 1;
                continue;
            }
            if blank(c) && !quoted {
                break;
            }
            arg.push(line[i]);
            i += 1;
        }
        args.push((arg, had_quote));
    }
    args
}

/// The entries of an environment block (`NAME=value` strings, each NUL
/// terminated, the block ended by an empty one) that `_environ` shows.
pub fn environment<C: Unit>(block: &[C]) -> Vec<&[C]> {
    block.split(|c| c.is_nul()).take_while(|e| !e.is_empty()).filter(|e| !is(e[0].to_u32(), b'=')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn argv(s: &str) -> Vec<String> {
        parse(s.as_bytes()).into_iter().map(|a| String::from_utf8(a).unwrap()).collect()
    }

    #[test]
    fn learns_examples() {
        // The table of "Parsing C command-line arguments".
        assert_eq!(argv(r#"p "abc" d e"#), ["p", "abc", "d", "e"]);
        assert_eq!(argv(r#"p a\\b d"e f"g h"#), ["p", r"a\\b", "de fg", "h"]);
        assert_eq!(argv(r#"p a\\\"b c d"#), ["p", r#"a\"b"#, "c", "d"]);
        assert_eq!(argv(r#"p a\\\\"b c" d e"#), ["p", r"a\\b c", "d", "e"]);
        assert_eq!(argv(r#"p a"b"" c d"#), ["p", r#"ab" c d"#]);
    }

    #[test]
    fn the_program_name() {
        assert_eq!(argv(r#""C:\Program Files\x\a.exe" -v"#), [r"C:\Program Files\x\a.exe", "-v"]);
        assert_eq!(argv(r#"C:\a\"b c"#), [r"C:\a\b c"]);
        assert_eq!(argv(" lead"), ["", "lead"]);
        assert_eq!(argv(""), [""]);
        assert_eq!(argv("prog"), ["prog"]);
    }

    #[test]
    fn quoting_details() {
        assert_eq!(argv("p \"\" x"), ["p", "", "x"]);
        assert_eq!(argv("p \"unterminated arg"), ["p", "unterminated arg"]);
        assert_eq!(argv("p\ta\t\tb  "), ["p", "a", "b"]);
        assert_eq!(argv(r"p trailing\\"), ["p", r"trailing\\"]);
        assert_eq!(argv(r#"p \"x\""#), ["p", r#""x""#]);
        let w: Vec<u16> = "p \"\u{e9}t\u{e9} x\" y".encode_utf16().collect();
        let a = parse(&w);
        assert_eq!(String::from_utf16(&a[1]).unwrap(), "\u{e9}t\u{e9} x");
    }

    #[test]
    fn quotes_are_marked() {
        let marks: Vec<bool> = parse_marked(br#"p *.c "*.h" a\"b x"y"z"#).into_iter().map(|(_, q)| q).collect();
        assert_eq!(marks, [false, false, true, false, true]);
        assert!(parse_marked(br#""C:\p" x"#)[0].1);
    }

    #[test]
    fn environment_entries() {
        let block = b"=C:=C:\\x\0A=1\0Path=C:\\Windows\0\0junk";
        let e = environment(block);
        assert_eq!(e, [&b"A=1"[..], &b"Path=C:\\Windows"[..]]);
        assert!(environment::<u8>(b"\0").is_empty());
    }
}
