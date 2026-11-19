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

/// What starts a framed result line on the kernel log (ADR 0228).
pub const RESULT_FRAME: &str = "POLY-RESULT ";

/// The result lines of a serial log from their framed copies (ADR 0228): every
/// `POLY-RESULT <text> crc32=<8 hex digits>` whose CRC-32 matches its text, anywhere in a
/// line (a kernel log line can start in the middle of a program's row), in order, and how
/// many frames had a bad or missing checksum (torn: never trusted).
pub fn result_frames(log: &str) -> (Vec<String>, usize) {
    let (mut good, mut bad) = (Vec::new(), 0);
    for line in log.lines().filter(|l| l.contains(RESULT_FRAME)) {
        let line = strip_ansi(line);
        let mut rest = line.as_str();
        while let Some(at) = rest.find(RESULT_FRAME) {
            let frame = &rest[at + RESULT_FRAME.len()..];
            // The frame's end: its checksum's eight digits.
            let parsed = frame.find(" crc32=").and_then(|c| {
                let digits = frame.get(c + 7..c + 15)?;
                let crc = u32::from_str_radix(digits, 16).ok().filter(|_| digits.len() == 8)?;
                Some((&frame[..c], crc, c + 15))
            });
            match parsed {
                Some((text, crc, end)) if crate::unzip::crc32(text.as_bytes()) == crc => {
                    good.push(text.to_string());
                    rest = &frame[end..];
                }
                _ => {
                    bad += 1;
                    rest = frame;
                }
            }
        }
    }
    (good, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(text: &str) -> String {
        format!("POLY-RESULT {text} crc32={:08x}", crate::unzip::crc32(text.as_bytes()))
    }

    #[test]
    fn result_frames_are_found_whole_and_checked() {
        let sha = "LAUNCH-SHA256 stdout 4a9c33a1d5c39feb2ab70d5cea5a2bfbaa36d7cc5f9bcdc8d4ff2ad8e5c8dcd0";
        let log = format!(
            "boot\r\n\x1b[6;1HLAUNCH-SHA256 stdout 4a9cnt: pid 1: {}\r\nnt: pid 1: {}\r\n\
             nt: pid 1: {}\r\n",
            frame(sha),
            frame("LAUNCH-EXIT code=0x0"),
            frame("LAUNCH-DIGEST stdout bytes=440"),
        );
        let (good, bad) = result_frames(&log);
        assert_eq!(good, [sha, "LAUNCH-EXIT code=0x0", "LAUNCH-DIGEST stdout bytes=440"]);
        assert_eq!(bad, 0);
    }

    #[test]
    fn torn_or_altered_frames_are_not_trusted() {
        let whole = frame("LAUNCH-DIGEST stdout bytes=440");
        // A kernel line inside the frame (as if it had been written in pieces).
        let torn = format!("{}fs: noise\r\n{}", &whole[..30], &whole[30..]);
        // One digit changed.
        let altered = whole.replace("440", "441");
        // Cut off before the checksum's last digit.
        let short = &whole[..whole.len() - 1];
        for log in [torn.as_str(), altered.as_str(), short] {
            let (good, bad) = result_frames(log);
            assert!(good.is_empty(), "{log:?}");
            assert!(bad >= 1, "{log:?}");
        }
        // Two frames on one line are both read.
        let (good, bad) = result_frames(&format!("{}{}", frame("a"), frame("b")));
        assert_eq!((good, bad), (vec!["a".to_string(), "b".to_string()], 0));
    }

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
