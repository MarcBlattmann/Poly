//! Reading the kernel shell's serial output (kernel/src/shell): the text
//! without its escape sequences, and the output of each command line.
//!
//! ```text
//! poly> uptime                     <- prompt and the line as it was drawn
//! up 0:00:01.234 (...) ticks ...   <- the command's output, up to the next prompt
//! poly> _
//! ```

/// The prompt as it reads once its colors are removed.
pub const PROMPT: &str = "poly> ";

/// `s` without ECMA-48 control sequences (`ESC [` parameters, final byte)
/// and carriage returns: the characters a terminal would show, in the order
/// they were written. Cursor movements are dropped too, so a line edited
/// in place reads as everything that was typed and redrawn.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                if chars.next() == Some('[') {
                    // Parameters and intermediates, up to the final byte.
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// One command line of a session: the row the prompt was on (after the
/// prompt) and everything printed after it up to the next prompt.
#[derive(Debug, PartialEq, Eq)]
pub struct Exchange<'a> {
    pub line: &'a str,
    pub output: &'a str,
}

/// Every command line in `text` (already through [`strip_ansi`]), in
/// order. Whatever comes before the first prompt (the boot log) is skipped.
pub fn exchanges(text: &str) -> Vec<Exchange<'_>> {
    text.split(PROMPT)
        .skip(1)
        .map(|part| match part.split_once('\n') {
            Some((line, output)) => Exchange { line, output },
            None => Exchange { line: part, output: "" },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colors_movements_and_carriage_returns() {
        assert_eq!(strip_ansi("\x1b[1;32mpoly>\x1b[0m ls\x1b[D\x1b[K\r\n"), "poly> ls\n");
        assert_eq!(strip_ansi("a\x1b[12Cb\x1b[2J\x1b[Hc"), "abc");
        assert_eq!(strip_ansi("ä€\x1b"), "ä€", "a sequence cut off at the end");
        assert_eq!(strip_ansi("\x1bxy"), "y", "ESC and the byte after it");
    }

    #[test]
    fn splits_a_session_into_exchanges() {
        let text = "boot log\npoly> help\nhelp text\nmore\npoly> \npoly> hlep\nunknown\npoly> ";
        assert_eq!(
            exchanges(text),
            [
                Exchange { line: "help", output: "help text\nmore\n" },
                Exchange { line: "", output: "" },
                Exchange { line: "hlep", output: "unknown\n" },
                Exchange { line: "", output: "" },
            ]
        );
        assert!(exchanges("no shell here\n").is_empty());
    }
}
