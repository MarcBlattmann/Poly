//! `cmd.exe`'s own command line (Learn: "cmd"):
//! `cmd [/c|/k] [/s] [/q] [/d] [/a|/u] [/t:{bf}] [/e:{on|off}] [/f:{on|off}] [/v:{on|off}] [<string>]`.
//!
//! The switches come before `/c` or `/k`; everything after `/c` or `/k` is the command
//! (the switch may be followed by the command without a space). [`command_quotes`] is the
//! quote rule Learn gives for that string.

use alloc::string::String;

/// What `cmd.exe` does after reading its command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No `/c` or `/k`: read commands from standard input.
    Interactive,
    /// `/c`: carry out the command, then stop.
    Run,
    /// `/k`: carry out the command, then read commands from standard input.
    RunThenStay,
}

/// `cmd.exe`'s switches and command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub mode: Mode,
    /// The command after `/c` or `/k`, as written (the quote rule not applied yet).
    pub command: String,
    /// `/s`: the quote rule always strips the outer quotes.
    pub strip_quotes: bool,
    /// `/q`: echo off.
    pub echo_off: bool,
    /// `/u`: output to files and pipes is UTF-16 (else ANSI, `/a`).
    pub unicode: bool,
    /// `/v:on`: delayed expansion of `!VAR!`.
    pub delayed: bool,
    /// `/e:off` (or `/y`): command extensions off.
    pub extensions: bool,
}

/// The rest of a Windows command line after its program name, by the rule for the first
/// argument (Learn: "CommandLineToArgvW", "Parsing C++ command-line arguments"): a quoted
/// name runs to the next quote, an unquoted one to the first space or tab.
pub fn after_program_name(line: &str) -> &str {
    let line = line.trim_start_matches([' ', '\t']);
    let end = if let Some(rest) = line.strip_prefix('"') {
        rest.find('"').map_or(line.len(), |i| i + 2)
    } else {
        line.find([' ', '\t']).unwrap_or(line.len())
    };
    &line[end..]
}

/// `cmd.exe`'s switches and command from its command line without the program name.
pub fn parse(args: &str) -> Invocation {
    let mut inv = Invocation {
        mode: Mode::Interactive,
        command: String::new(),
        strip_quotes: false,
        echo_off: false,
        unicode: false,
        delayed: false,
        extensions: true,
    };
    let mut rest = args;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        let Some(sw) = rest.strip_prefix('/') else {
            // Text that is no switch and comes before /c or /k: ignored.
            match rest.find([' ', '\t']) {
                Some(i) => {
                    rest = &rest[i..];
                    continue;
                }
                None => break,
            }
        };
        let letter = sw.chars().next().map(|c| c.to_ascii_lowercase());
        let token_end = sw.find([' ', '\t', '/']).unwrap_or(sw.len());
        let token = &sw[..token_end];
        match letter {
            Some('c') | Some('k') => {
                inv.mode = if letter == Some('c') { Mode::Run } else { Mode::RunThenStay };
                inv.command = String::from(sw[1..].trim_start_matches([' ', '\t']));
                break;
            }
            None => break,
            _ => {}
        }
        let value = token.split_once(':').map(|(_, v)| v);
        let on = |v: Option<&str>| v.is_none_or(|v| !crate::eq_ic(v, "off"));
        match letter {
            Some('s') => inv.strip_quotes = true,
            Some('q') => inv.echo_off = true,
            Some('u') => inv.unicode = true,
            Some('a') => inv.unicode = false,
            Some('v') => inv.delayed = on(value),
            Some('e') => inv.extensions = on(value),
            Some('x') => inv.extensions = true,
            Some('y') => inv.extensions = false,
            // /d (no AutoRun), /t:bf (colours), /f (completion): nothing to do.
            _ => {}
        }
        rest = &sw[token_end..];
    }
    inv
}

/// The characters that make the quote rule strip the quotes when they are between them.
const SPECIAL: &[char] = &['&', '<', '>', '(', ')', '@', '^', '|'];

/// The quote rule for the `/c` or `/k` string (Learn: "cmd", "Remarks"):
///
/// 1. The quotes stay when all of these hold: no `/s`; exactly two quote characters; no
///    special character (`& < > ( ) @ ^ |`) between them; at least one white-space character
///    between them; the text between them is the name of an executable file
///    (`is_executable`).
/// 2. Otherwise, if the first character is a quote, it is removed together with the last
///    quote of the line; the text after that last quote stays.
pub fn command_quotes(command: &str, strip: bool, is_executable: impl Fn(&str) -> bool) -> String {
    let command = command.trim_start_matches([' ', '\t']);
    if !strip && command.matches('"').count() == 2 {
        let first = command.find('"').unwrap_or(0);
        let last = command.rfind('"').unwrap_or(0);
        let inner = &command[first + 1..last];
        if !inner.contains(SPECIAL) && inner.contains([' ', '\t']) && is_executable(inner) {
            return String::from(command);
        }
    }
    if let Some(rest) = command.strip_prefix('"') {
        match rest.rfind('"') {
            Some(i) => {
                let mut s = String::from(&rest[..i]);
                s.push_str(&rest[i + 1..]);
                s
            }
            None => String::from(rest),
        }
    } else {
        String::from(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name_is_skipped() {
        assert_eq!(after_program_name("C:\\Windows\\System32\\cmd.exe /c exit"), " /c exit");
        assert_eq!(after_program_name("\"C:\\Program Files\\cmd.exe\" /c exit 3"), " /c exit 3");
        assert_eq!(after_program_name("  cmd"), "");
        assert_eq!(after_program_name("\"unterminated"), "");
    }

    #[test]
    fn c_and_k() {
        let i = parse(" /c exit 3");
        assert_eq!(i.mode, Mode::Run);
        assert_eq!(i.command, "exit 3");
        let i = parse("/K echo hi");
        assert_eq!(i.mode, Mode::RunThenStay);
        assert_eq!(i.command, "echo hi");
        // the command may follow the switch directly
        assert_eq!(parse("/cexit 4").command, "exit 4");
        assert_eq!(parse("").mode, Mode::Interactive);
        assert_eq!(parse("/q").mode, Mode::Interactive);
    }

    #[test]
    fn switches_before_c() {
        let i = parse("/d /q /s /v:on /u /t:0a /e:off /c \"x\"");
        assert!(i.echo_off && i.strip_quotes && i.delayed && i.unicode && !i.extensions);
        assert_eq!(i.command, "\"x\"");
        let i = parse("/V:OFF /E:ON /A /C exit");
        assert!(!i.delayed && i.extensions && !i.unicode);
        assert_eq!(i.command, "exit");
        // switches after /c are part of the command
        assert_eq!(parse("/c echo /q").command, "echo /q");
        // glued switches
        assert!(parse("/q/v:on/c x").delayed);
    }

    #[test]
    fn quote_rule() {
        let exe = |s: &str| s == "C:\\Program Files\\app.exe";
        // rule 1: kept
        assert_eq!(
            command_quotes("\"C:\\Program Files\\app.exe\" -x", false, exe),
            "\"C:\\Program Files\\app.exe\" -x"
        );
        // /s: stripped
        assert_eq!(command_quotes("\"C:\\Program Files\\app.exe\" -x", true, exe), "C:\\Program Files\\app.exe -x");
        // more than two quotes: first and last removed
        assert_eq!(
            command_quotes("\"\"C:\\Program Files\\app.exe\" \"a b\"\"", false, exe),
            "\"C:\\Program Files\\app.exe\" \"a b\""
        );
        // no white space between the quotes: stripped
        assert_eq!(command_quotes("\"exit\" 3", false, |_| true), "exit 3");
        // not an executable: stripped, text after the last quote kept
        assert_eq!(command_quotes("\"echo a b\" tail", false, |_| false), "echo a b tail");
        // a special character between the quotes
        assert_eq!(command_quotes("\"echo a & echo b\"", false, |_| true), "echo a & echo b");
        // no quote first: unchanged
        assert_eq!(command_quotes("exit \"%1\"", false, |_| true), "exit \"%1\"");
        assert_eq!(command_quotes("  \"exit 2", false, |_| false), "exit 2");
    }
}
