//! Variable expansion (Learn: "set", "Using batch parameters", "cmd" on delayed expansion).
//!
//! `%NAME%` is replaced by the variable's value before a line is parsed. `%NAME:~s,l%` takes
//! a substring (a negative start counts from the end, a negative length leaves that many
//! characters off the end); `%NAME:old=new%` replaces every `old` (without case) and
//! `%NAME:*old=new%` replaces everything up to and including the first `old`.
//!
//! On a command line a variable that is not defined stays as written; in a batch file it
//! becomes empty, `%%` becomes `%`, `%0`..`%9` are the batch parameters, `%*` all of them from
//! the first, and `%~…n` a parameter without quotes or a part of it as a path (`f` full path,
//! `d` drive, `p` path, `n` name, `x` extension).
//!
//! With delayed expansion, `!NAME!` is expanded the same way when a command runs.

use alloc::string::String;
use alloc::vec::Vec;

/// The parameters of the batch file that is running.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchArgs {
    /// `%0`: the batch file as it was named.
    pub zero: String,
    /// `%0` as a full path (for `%~f0`, `%~dp0`, …).
    pub full: String,
    /// `%1`, `%2`, …, each as written (quotes kept).
    pub args: Vec<String>,
    /// How many `shift`s ran.
    pub shift: usize,
}

impl BatchArgs {
    /// Parameter `n` after the shifts (0 is the batch file or the shifted-in argument).
    pub fn get(&self, n: usize) -> Option<&str> {
        let k = n + self.shift;
        if k == 0 { Some(&self.zero) } else { self.args.get(k - 1).map(String::as_str) }
    }

    /// `%*`: all the arguments from the first, as written, separated by spaces (shifts do not
    /// change it, Learn: "shift").
    pub fn all(&self) -> String {
        self.args.join(" ")
    }
}

/// Split batch arguments as `cmd.exe` does (Learn: "Using batch parameters"): separated by
/// spaces, tabs, commas, semicolons or equal signs, a quoted part keeps its separators and its
/// quotes.
pub fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
            cur.push(c);
        } else if !quoted && matches!(c, ' ' | '\t' | ',' | ';' | '=') {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A substring or substitution spec (`~s,l`, `old=new`, `*old=new`) applied to `value`;
/// `None` when the spec is not one of them.
fn modify(value: &str, spec: &str) -> Option<String> {
    let chars: Vec<char> = value.chars().collect();
    let len = chars.len() as i64;
    if let Some(range) = spec.strip_prefix('~') {
        let (s, l) = match range.split_once(',') {
            Some((s, l)) => (s.trim().parse::<i64>().ok()?, Some(l.trim().parse::<i64>().ok()?)),
            None => (range.trim().parse::<i64>().ok()?, None),
        };
        let start = if s < 0 { (len + s).max(0) } else { s.min(len) };
        let end = match l {
            None => len,
            Some(l) if l < 0 => (len + l).max(start),
            Some(l) => (start + l).min(len),
        };
        return Some(chars[start as usize..end.max(start) as usize].iter().collect());
    }
    let (old, new) = spec.split_once('=')?;
    if let Some(old) = old.strip_prefix('*') {
        if old.is_empty() {
            return Some(String::from(value));
        }
        return Some(match find_ic(value, old, 0) {
            Some(i) => {
                let mut s = String::from(new);
                s.push_str(&value[i + old.len()..]);
                s
            }
            None => String::from(value),
        });
    }
    if old.is_empty() {
        return Some(String::from(value));
    }
    let mut s = String::new();
    let mut at = 0;
    while let Some(i) = find_ic(value, old, at) {
        s.push_str(&value[at..i]);
        s.push_str(new);
        at = i + old.len();
    }
    s.push_str(&value[at..]);
    Some(s)
}

/// The byte offset of `needle` in `hay` at or after `from`, ASCII case ignored.
fn find_ic(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    (from..=h.len().checked_sub(n.len())?)
        .find(|&i| hay.is_char_boundary(i) && h[i..i + n.len()].eq_ignore_ascii_case(n))
}

/// A path's parts for the `%~` modifiers: (drive `C:`, directory with its trailing `\`,
/// name, extension with its dot).
fn path_parts(full: &str) -> (&str, &str, &str, &str) {
    let (drive, rest) = if full.len() >= 2 && full.as_bytes()[1] == b':' { full.split_at(2) } else { ("", full) };
    let cut = rest.rfind(['\\', '/']).map_or(0, |i| i + 1);
    let (dir, file) = rest.split_at(cut);
    match file.rfind('.') {
        Some(i) => (drive, dir, &file[..i], &file[i..]),
        None => (drive, dir, file, ""),
    }
}

/// `%~<mods><n>` for one parameter.
fn tilde(value: &str, full: Option<&str>, mods: &str) -> String {
    let unquoted = value.trim_matches('"');
    if mods.is_empty() {
        return String::from(unquoted);
    }
    let f = full.unwrap_or(unquoted);
    let (d, p, n, x) = path_parts(f);
    if mods.contains(['f', 'F']) && !mods.contains(['d', 'D', 'p', 'P', 'n', 'N', 'x', 'X']) {
        return String::from(f);
    }
    let mut s = String::new();
    for (letter, part) in [('d', d), ('p', p), ('n', n), ('x', x)] {
        if mods.chars().any(|c| c.eq_ignore_ascii_case(&letter)) {
            s.push_str(part);
        }
    }
    s
}

/// Where a line is expanded.
#[allow(missing_debug_implementations)]
pub struct Context<'a> {
    /// The parameters of the running batch file; `None` on the command line.
    pub batch: Option<&'a BatchArgs>,
    /// A variable's value (the shell's dynamic variables included).
    pub var: &'a dyn Fn(&str) -> Option<String>,
    /// The full path of an argument (for `%~f1`); `None` leaves it as written.
    pub full_path: &'a dyn Fn(&str) -> Option<String>,
}

/// `%…%` expansion of one line.
pub fn percent(line: &str, cx: &Context<'_>) -> String {
    expand(line, '%', cx)
}

/// `!…!` expansion (delayed expansion) of one command; batch parameters do not apply.
pub fn delayed(line: &str, cx: &Context<'_>) -> String {
    if !line.contains('!') {
        return String::from(line);
    }
    expand_vars(line, '!', cx, false)
}

fn expand(line: &str, mark: char, cx: &Context<'_>) -> String {
    expand_vars(line, mark, cx, true)
}

fn expand_vars(line: &str, mark: char, cx: &Context<'_>, params: bool) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let in_batch = cx.batch.is_some();
    while i < chars.len() {
        let c = chars[i];
        if c != mark {
            out.push(c);
            i += 1;
            continue;
        }
        if params && let Some(b) = cx.batch {
            match chars.get(i + 1) {
                Some('%') => {
                    out.push('%');
                    i += 2;
                    continue;
                }
                Some(d) if d.is_ascii_digit() => {
                    out.push_str(b.get(*d as usize - '0' as usize).unwrap_or(""));
                    i += 2;
                    continue;
                }
                Some('*') => {
                    out.push_str(&b.all());
                    i += 2;
                    continue;
                }
                Some('~') => {
                    let mut j = i + 2;
                    while j < chars.len() && chars[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if let Some(d) = chars.get(j).filter(|d| d.is_ascii_digit()) {
                        let mods: String = chars[i + 2..j].iter().collect();
                        let n = *d as usize - '0' as usize;
                        let value = b.get(n).unwrap_or("");
                        let full = if n == 0 && b.shift == 0 {
                            Some(String::from(b.full.as_str()))
                        } else if value.is_empty() {
                            None
                        } else {
                            (cx.full_path)(value.trim_matches('"'))
                        };
                        out.push_str(&tilde(value, full.as_deref(), &mods));
                        i = j + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = chars[i + 1..].iter().position(|&x| x == mark).map(|p| p + i + 1) else {
            // A lone mark: dropped in a batch file, kept on the command line.
            if !in_batch {
                out.push(c);
            }
            i += 1;
            continue;
        };
        let inner: String = chars[i + 1..close].iter().collect();
        let (name, spec) = match inner.split_once(':') {
            Some((n, s)) => (n, Some(s)),
            None => (inner.as_str(), None),
        };
        let value = if name.is_empty() { None } else { (cx.var)(name) };
        let value = match (value, spec) {
            (Some(v), Some(s)) => modify(&v, s),
            (v, None) => v,
            (None, Some(_)) => None,
        };
        match value {
            Some(v) => {
                out.push_str(&v);
                i = close + 1;
            }
            None if in_batch => i = close + 1,
            None => {
                // Not defined on the command line: the text stays, and the closing mark
                // may open the next variable.
                out.push(c);
                out.push_str(&inner);
                i = close;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn vars(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "A" => Some("hello".into()),
            "PATH" => Some("C:\\Windows\\System32;C:\\Tools".into()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    fn full(p: &str) -> Option<String> {
        Some(alloc::format!("C:\\work\\{p}"))
    }

    fn line(s: &str) -> String {
        percent(s, &Context { batch: None, var: &vars, full_path: &full })
    }

    fn batch(s: &str, b: &BatchArgs) -> String {
        percent(s, &Context { batch: Some(b), var: &vars, full_path: &full })
    }

    #[test]
    fn command_line() {
        assert_eq!(line("echo %A% x"), "echo hello x");
        assert_eq!(line("echo %a%"), "echo hello");
        assert_eq!(line("echo %NOPE%"), "echo %NOPE%");
        assert_eq!(line("echo 100%"), "echo 100%");
        assert_eq!(line("echo %NOPE%A%"), "echo %NOPEhello");
        assert_eq!(line("[%EMPTY%]"), "[]");
        assert_eq!(line("exit \"%1\""), "exit \"%1\"");
        assert_eq!(line("%%"), "%%");
    }

    #[test]
    fn substrings_and_substitutions() {
        assert_eq!(line("%A:~1,3%"), "ell");
        assert_eq!(line("%A:~-2%"), "lo");
        assert_eq!(line("%A:~0,-2%"), "hel");
        assert_eq!(line("%A:~10%"), "");
        assert_eq!(line("%A:L=x%"), "hexxo");
        assert_eq!(line("%A:*l=%"), "lo");
        assert_eq!(line("%PATH:;=,%"), "C:\\Windows\\System32,C:\\Tools");
        assert_eq!(line("%NOPE:~1%"), "%NOPE:~1%");
    }

    #[test]
    fn batch_parameters() {
        let b = BatchArgs {
            zero: "run.bat".into(),
            full: "C:\\scripts\\run.bat".into(),
            args: vec!["one".into(), "\"two words\"".into(), "x.txt".into()],
            shift: 0,
        };
        assert_eq!(batch("%0 %1 %2 %3 %4", &b), "run.bat one \"two words\" x.txt ");
        assert_eq!(batch("%*", &b), "one \"two words\" x.txt");
        assert_eq!(batch("%~2", &b), "two words");
        assert_eq!(batch("%~dp0", &b), "C:\\scripts\\");
        assert_eq!(batch("%~nx0|%~n0|%~x0|%~d0|%~f0", &b), "run.bat|run|.bat|C:|C:\\scripts\\run.bat");
        assert_eq!(batch("%~f3", &b), "C:\\work\\x.txt");
        assert_eq!(batch("100%% %NOPE%end %A%", &b), "100% end hello");
        assert_eq!(batch("a % b", &b), "a  b");
        let mut s = b.clone();
        s.shift = 1;
        assert_eq!(batch("%0 %1", &s), "one \"two words\"");
    }

    #[test]
    fn delayed_expansion() {
        let cx = Context { batch: None, var: &vars, full_path: &full };
        assert_eq!(delayed("echo !A!", &cx), "echo hello");
        assert_eq!(delayed("echo !NOPE!", &cx), "echo !NOPE!");
        assert_eq!(delayed("echo %A%", &cx), "echo %A%");
    }

    #[test]
    fn splitting_batch_args() {
        assert_eq!(split_args(" a,b;c=d  \"e f\"\tg"), vec!["a", "b", "c", "d", "\"e f\"", "g"]);
        assert!(split_args("").is_empty());
    }
}
