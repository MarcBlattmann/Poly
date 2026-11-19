//! A command line (after `%` expansion) as a tree (Learn: "Command shell overview",
//! "Using command redirection operators", "if").
//!
//! - `a & b` runs both, `a && b` runs `b` when `a` succeeded, `a || b` when it failed,
//!   `a | b` pipes; `&` binds loosest, `&&` and `||` go left to right, `|` binds tightest.
//! - `( … )` groups commands; inside it a line break separates commands like `&`, and `)`
//!   ends the group (outside a group `)` is an ordinary character).
//! - `^` takes the next character as it is (outside quotes); a quoted part keeps every
//!   character and its quotes.
//! - `<file`, `>file`, `>>file`, with an optional handle number in front (`2>file`), and
//!   `>&n` (`2>&1`): anywhere in a command, applied left to right.
//! - `if [/i] [not] errorlevel n | exist file | defined name | a==b | a op b  command
//!   [else command]`: without a group, the command is the rest of the line; `else` follows
//!   only the `)` of a group.
//! - `@` in front of a command hides it from echo; `rem` takes the rest of the line.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Nothing to run.
    Empty,
    /// A command: a built-in or a program.
    Command(Command),
    /// `( … )` and the redirections after it.
    Block(Box<Node>, Vec<Redir>),
    /// `if …`.
    If(Box<If>),
    /// `a & b` (or a line break in a group).
    Then(Box<Node>, Box<Node>),
    /// `a && b`.
    And(Box<Node>, Box<Node>),
    /// `a || b`.
    Or(Box<Node>, Box<Node>),
    /// `a | b`.
    Pipe(Box<Node>, Box<Node>),
}

/// One command: its text without the redirections (quotes kept, `^` removed) and its
/// redirections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub text: String,
    pub redirs: Vec<Redir>,
    /// `@` in front: not echoed.
    pub quiet: bool,
}

/// A redirection of a handle (0 input, 1 output, 2 errors).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redir {
    pub handle: u8,
    pub to: Target,
}

/// Where a redirected handle goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// `<file`.
    Read(String),
    /// `>file`.
    Write(String),
    /// `>>file`.
    Append(String),
    /// `>&n`: a copy of handle `n`.
    Handle(u8),
}

/// `if`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct If {
    pub not: bool,
    /// `/i`: string comparisons ignore case.
    pub ignore_case: bool,
    pub cond: Cond,
    pub then: Node,
    pub otherwise: Option<Node>,
}

/// The condition of an `if`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    /// `errorlevel n`: the exit code of the last program is `n` or more.
    ErrorLevel(i64),
    /// `exist file`.
    Exist(String),
    /// `defined name`.
    Defined(String),
    /// `cmdextversion n`.
    CmdExtVersion(i64),
    /// `a==b` or `a op b`; the operands keep their quotes.
    Compare(String, Op, String),
}

/// A comparison of `if` (`==` and, with command extensions, the three-letter operators).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `==`: the strings are equal.
    Same,
    Equ,
    Neq,
    Lss,
    Leq,
    Gtr,
    Geq,
}

/// Why a line does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A `(` is not closed: the line continues on the next one (batch files).
    Unclosed,
    /// Anything else, with what was wrong.
    Syntax(&'static str),
}

/// Parse a command line.
pub fn parse(line: &str) -> Result<Node, Error> {
    let mut p = Parser { s: line.chars().collect(), i: 0, depth: 0 };
    let node = p.seq(false)?;
    p.blank();
    if p.i < p.s.len() {
        return Err(Error::Syntax("unexpected text"));
    }
    Ok(node)
}

struct Parser {
    s: Vec<char>,
    i: usize,
    /// How many groups are open.
    depth: usize,
}

impl Parser {
    fn at(&self, k: usize) -> Option<char> {
        self.s.get(self.i + k).copied()
    }

    fn peek(&self) -> Option<char> {
        self.at(0)
    }

    /// Skip spaces and tabs.
    fn blank(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.i += 1;
        }
    }

    /// The word at the cursor (letters, digits and a few signs), not consumed.
    fn peek_word(&self) -> String {
        self.s[self.i..]
            .iter()
            .take_while(|c| !matches!(c, ' ' | '\t' | '\n' | '&' | '|' | '<' | '>' | '(' | ')' | '"' | '^'))
            .collect()
    }

    /// Whether the word at the cursor is `kw` (without case) and ends there.
    fn keyword(&self, kw: &str) -> bool {
        let n = kw.chars().count();
        let word: String = self.s[self.i..].iter().take(n).collect();
        crate::eq_ic(&word, kw) && matches!(self.at(n), None | Some(' ' | '\t' | '\n' | '('))
    }

    /// `a & b & …`; a line break separates like `&` unless `line` (the body of an `if`
    /// without a group ends at the end of its line).
    fn seq(&mut self, line: bool) -> Result<Node, Error> {
        let mut left = self.cond()?;
        loop {
            self.blank();
            match self.peek() {
                Some('&') if self.at(1) != Some('&') => self.i += 1,
                Some('\n') if !line => self.i += 1,
                _ => break,
            }
            let right = self.cond()?;
            left = join(left, right, Node::Then);
        }
        Ok(left)
    }

    /// `a && b || c …`, left to right.
    fn cond(&mut self) -> Result<Node, Error> {
        let mut left = self.pipe()?;
        loop {
            self.blank();
            let and = match (self.peek(), self.at(1)) {
                (Some('&'), Some('&')) => true,
                (Some('|'), Some('|')) => false,
                _ => break,
            };
            self.i += 2;
            let right = self.pipe()?;
            if right == Node::Empty {
                return Err(Error::Syntax("a command must follow && or ||"));
            }
            left = if and {
                Node::And(Box::new(left), Box::new(right))
            } else {
                Node::Or(Box::new(left), Box::new(right))
            };
        }
        Ok(left)
    }

    /// `a | b | …`.
    fn pipe(&mut self) -> Result<Node, Error> {
        let mut left = self.command()?;
        loop {
            self.blank();
            if self.peek() != Some('|') || self.at(1) == Some('|') {
                break;
            }
            self.i += 1;
            let right = self.command()?;
            if left == Node::Empty || right == Node::Empty {
                return Err(Error::Syntax("a pipe needs a command on both sides"));
            }
            left = Node::Pipe(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// One command, group or `if`.
    fn command(&mut self) -> Result<Node, Error> {
        self.blank();
        let mut quiet = false;
        while self.peek() == Some('@') {
            quiet = true;
            self.i += 1;
            self.blank();
        }
        match self.peek() {
            None | Some('&' | '|' | '\n') => return Ok(Node::Empty),
            Some(')') if self.depth > 0 => return Ok(Node::Empty),
            Some('(') => return self.group(),
            _ => {}
        }
        if self.keyword("if") {
            self.i += 2;
            return self.if_command();
        }
        if self.keyword("rem") {
            let start = self.i;
            while !matches!(self.peek(), None | Some('\n')) {
                self.i += 1;
            }
            let text = self.s[start..self.i].iter().collect();
            return Ok(Node::Command(Command { text, redirs: Vec::new(), quiet }));
        }
        self.simple(quiet)
    }

    /// `( … )` and its redirections.
    fn group(&mut self) -> Result<Node, Error> {
        self.i += 1;
        self.depth += 1;
        let body = self.seq(false)?;
        self.blank();
        match self.peek() {
            Some(')') => self.i += 1,
            None => return Err(Error::Unclosed),
            Some(_) => return Err(Error::Syntax("unexpected text in a group")),
        }
        self.depth -= 1;
        let mut redirs = Vec::new();
        loop {
            self.blank();
            match self.peek() {
                Some('<' | '>') => redirs.push(self.redir(None)?),
                Some(d) if d.is_ascii_digit() && matches!(self.at(1), Some('<' | '>')) => {
                    self.i += 1;
                    redirs.push(self.redir(Some(d as u8 - b'0'))?);
                }
                _ => break,
            }
        }
        Ok(Node::Block(Box::new(body), redirs))
    }

    /// Whether `c` ends a word or a command at the cursor (outside quotes).
    fn ends(&self, c: char) -> bool {
        matches!(c, '&' | '|' | '\n') || (c == ')' && self.depth > 0)
    }

    /// A command: text and redirections up to the next operator.
    fn simple(&mut self, quiet: bool) -> Result<Node, Error> {
        let mut text = String::new();
        let mut redirs = Vec::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            if quoted {
                text.push(c);
                self.i += 1;
                quoted = c != '"';
                continue;
            }
            match c {
                '"' => {
                    quoted = true;
                    text.push(c);
                    self.i += 1;
                }
                '^' => {
                    self.i += 1;
                    if let Some(n) = self.peek() {
                        self.i += 1;
                        if n != '\n' {
                            text.push(n);
                        }
                    }
                }
                '<' | '>' => {
                    redirs.push(self.redir(None)?);
                    self.after_redir(&text);
                }
                d if d.is_ascii_digit()
                    && matches!(self.at(1), Some('<' | '>'))
                    && (text.is_empty() || text.ends_with([' ', '\t'])) =>
                {
                    self.i += 1;
                    redirs.push(self.redir(Some(d as u8 - b'0'))?);
                    self.after_redir(&text);
                }
                c if self.ends(c) => break,
                _ => {
                    text.push(c);
                    self.i += 1;
                }
            }
        }
        if text.trim().is_empty() && redirs.is_empty() {
            return Ok(Node::Empty);
        }
        Ok(Node::Command(Command { text, redirs, quiet }))
    }

    /// After a redirection is taken out of a command, the blanks that follow it go too when
    /// the text so far is empty or already ends with one (`>f echo a`, `echo a > f 2>&1`).
    fn after_redir(&mut self, text: &str) {
        if text.is_empty() || text.ends_with([' ', '\t']) {
            self.blank();
        }
    }

    /// A redirection at the cursor (`<`, `>` or `>>`; the handle number already read).
    fn redir(&mut self, handle: Option<u8>) -> Result<Redir, Error> {
        let input = self.peek() == Some('<');
        self.i += 1;
        let append = !input && self.peek() == Some('>');
        if append {
            self.i += 1;
        }
        let handle = handle.unwrap_or(if input { 0 } else { 1 });
        if self.peek() == Some('&')
            && let Some(d) = self.at(1).filter(char::is_ascii_digit)
        {
            self.i += 2;
            return Ok(Redir { handle, to: Target::Handle(d as u8 - b'0') });
        }
        self.blank();
        let name = self.file_name();
        if name.is_empty() {
            return Err(Error::Syntax("a redirection needs a file name"));
        }
        let to = if input {
            Target::Read(name)
        } else if append {
            Target::Append(name)
        } else {
            Target::Write(name)
        };
        Ok(Redir { handle, to })
    }

    /// The file name of a redirection, quotes removed.
    fn file_name(&mut self) -> String {
        let mut name = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if c == '"' {
                quoted = !quoted;
                self.i += 1;
                continue;
            }
            if !quoted && (matches!(c, ' ' | '\t' | '<' | '>') || self.ends(c)) || c == '\n' {
                break;
            }
            if c == '^' && !quoted {
                self.i += 1;
                if let Some(n) = self.peek() {
                    name.push(n);
                    self.i += 1;
                }
                continue;
            }
            name.push(c);
            self.i += 1;
        }
        name
    }

    /// An operand of `if`: up to white space (or `==` when `eq`), quoted parts kept whole
    /// with their quotes.
    fn operand(&mut self, eq: bool) -> String {
        let mut s = String::new();
        let mut quoted = false;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            if !quoted {
                if matches!(c, ' ' | '\t' | '<' | '>' | '(') || self.ends(c) {
                    break;
                }
                if eq && c == '=' && self.at(1) == Some('=') {
                    break;
                }
                if c == '^' {
                    self.i += 1;
                    if let Some(n) = self.peek() {
                        s.push(n);
                        self.i += 1;
                    }
                    continue;
                }
            }
            if c == '"' {
                quoted = !quoted;
            }
            s.push(c);
            self.i += 1;
        }
        s
    }

    /// `if` after its keyword.
    fn if_command(&mut self) -> Result<Node, Error> {
        let mut not = false;
        let mut ignore_case = false;
        loop {
            self.blank();
            if self.keyword("/i") {
                ignore_case = true;
                self.i += 2;
            } else if self.keyword("not") {
                not = true;
                self.i += 3;
            } else {
                break;
            }
        }
        let number = |s: &str| crate::arith::number(s).ok_or(Error::Syntax("if needs a number"));
        let word = self.peek_word();
        let cond = if self.keyword("errorlevel") {
            self.i += word.chars().count();
            self.blank();
            Cond::ErrorLevel(number(&self.operand(false))?)
        } else if self.keyword("cmdextversion") {
            self.i += word.chars().count();
            self.blank();
            Cond::CmdExtVersion(number(&self.operand(false))?)
        } else if self.keyword("exist") {
            self.i += word.chars().count();
            self.blank();
            Cond::Exist(self.operand(false))
        } else if self.keyword("defined") {
            self.i += word.chars().count();
            self.blank();
            Cond::Defined(self.operand(false))
        } else {
            let a = self.operand(true);
            self.blank();
            let op = if self.peek() == Some('=') && self.at(1) == Some('=') {
                self.i += 2;
                Op::Same
            } else {
                let w = self.peek_word();
                let op = match w.to_ascii_uppercase().as_str() {
                    "EQU" => Op::Equ,
                    "NEQ" => Op::Neq,
                    "LSS" => Op::Lss,
                    "LEQ" => Op::Leq,
                    "GTR" => Op::Gtr,
                    "GEQ" => Op::Geq,
                    _ => return Err(Error::Syntax("if needs a comparison")),
                };
                self.i += w.chars().count();
                op
            };
            self.blank();
            let b = self.operand(false);
            if a.is_empty() || b.is_empty() {
                return Err(Error::Syntax("if needs two operands"));
            }
            Cond::Compare(a, op, b)
        };
        self.blank();
        let (then, grouped) = self.branch()?;
        if then == Node::Empty {
            return Err(Error::Syntax("if needs a command"));
        }
        let mut otherwise = None;
        if grouped {
            let save = self.i;
            self.blank();
            if self.keyword("else") {
                self.i += 4;
                self.blank();
                otherwise = Some(self.branch()?.0);
            } else {
                self.i = save;
            }
        }
        Ok(Node::If(Box::new(If { not, ignore_case, cond, then, otherwise })))
    }

    /// The command of `if` or `else`: a group, or the rest of the line; and whether it was a
    /// group.
    fn branch(&mut self) -> Result<(Node, bool), Error> {
        if self.peek() == Some('(') { Ok((self.command()?, true)) } else { Ok((self.seq(true)?, false)) }
    }
}

/// `a op b`, where an empty side drops out.
fn join(a: Node, b: Node, op: fn(Box<Node>, Box<Node>) -> Node) -> Node {
    match (a, b) {
        (Node::Empty, b) => b,
        (a, Node::Empty) => a,
        (a, b) => op(Box::new(a), Box::new(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn cmd(text: &str) -> Node {
        Node::Command(Command { text: text.into(), redirs: Vec::new(), quiet: false })
    }

    fn with(text: &str, redirs: Vec<Redir>) -> Node {
        Node::Command(Command { text: text.into(), redirs, quiet: false })
    }

    fn b(n: Node) -> Box<Node> {
        Box::new(n)
    }

    #[test]
    fn simple_commands() {
        assert_eq!(parse("exit 3"), Ok(cmd("exit 3")));
        assert_eq!(parse("   "), Ok(Node::Empty));
        assert_eq!(parse("echo  a  b "), Ok(cmd("echo  a  b ")));
        assert_eq!(parse("echo a^&b ^| \"x & y\""), Ok(cmd("echo a&b | \"x & y\"")));
        assert_eq!(parse("echo (hi)"), Ok(cmd("echo (hi)")));
        assert_eq!(parse("echo )"), Ok(cmd("echo )")));
        assert_eq!(
            parse("@echo off"),
            Ok(Node::Command(Command { text: "echo off".into(), redirs: Vec::new(), quiet: true }))
        );
        assert_eq!(parse("rem a & b > c"), Ok(cmd("rem a & b > c")));
    }

    #[test]
    fn operators() {
        assert_eq!(parse("a & b"), Ok(Node::Then(b(cmd("a ")), b(cmd("b")))));
        assert_eq!(parse("a&&b||c"), Ok(Node::Or(b(Node::And(b(cmd("a")), b(cmd("b")))), b(cmd("c")))));
        assert_eq!(parse("a | b & c"), Ok(Node::Then(b(Node::Pipe(b(cmd("a ")), b(cmd("b ")))), b(cmd("c")))));
        assert_eq!(parse("a &"), Ok(cmd("a ")));
        assert!(matches!(parse("a &&"), Err(Error::Syntax(_))));
        assert!(matches!(parse("| a"), Err(Error::Syntax(_))));
    }

    #[test]
    fn redirections() {
        assert_eq!(
            parse("echo hi > out.txt 2>&1"),
            Ok(with(
                "echo hi ",
                vec![
                    Redir { handle: 1, to: Target::Write("out.txt".into()) },
                    Redir { handle: 2, to: Target::Handle(1) }
                ]
            ))
        );
        assert_eq!(
            parse("prog <in.txt >>\"log file.txt\" 2>nul"),
            Ok(with(
                "prog ",
                vec![
                    Redir { handle: 0, to: Target::Read("in.txt".into()) },
                    Redir { handle: 1, to: Target::Append("log file.txt".into()) },
                    Redir { handle: 2, to: Target::Write("nul".into()) },
                ]
            ))
        );
        // a digit inside a word is text
        assert_eq!(parse("echo a2>x"), Ok(with("echo a2", vec![Redir { handle: 1, to: Target::Write("x".into()) }])));
        assert_eq!(parse(">x echo hi"), Ok(with("echo hi", vec![Redir { handle: 1, to: Target::Write("x".into()) }])));
        assert!(matches!(parse("echo >"), Err(Error::Syntax(_))));
        assert_eq!(parse("echo \"a>b\""), Ok(cmd("echo \"a>b\"")));
    }

    #[test]
    fn groups() {
        assert_eq!(
            parse("(a & b) > f"),
            Ok(Node::Block(
                b(Node::Then(b(cmd("a ")), b(cmd("b")))),
                vec![Redir { handle: 1, to: Target::Write("f".into()) }]
            ))
        );
        assert_eq!(parse("(a\nb\n)"), Ok(Node::Block(b(Node::Then(b(cmd("a")), b(cmd("b")))), Vec::new())));
        assert_eq!(parse("(echo a"), Err(Error::Unclosed));
        assert!(matches!(parse("(a) b"), Err(Error::Syntax(_))));
    }

    fn if_node(not: bool, ignore_case: bool, cond: Cond, then: Node, otherwise: Option<Node>) -> Node {
        Node::If(Box::new(If { not, ignore_case, cond, then, otherwise }))
    }

    #[test]
    fn if_forms() {
        assert_eq!(
            parse("if errorlevel 1 exit 5"),
            Ok(if_node(false, false, Cond::ErrorLevel(1), cmd("exit 5"), None))
        );
        assert_eq!(
            parse("IF NOT ERRORLEVEL 2 echo low & echo more"),
            Ok(if_node(true, false, Cond::ErrorLevel(2), Node::Then(b(cmd("echo low ")), b(cmd("echo more"))), None))
        );
        assert_eq!(
            parse("if /i \"%a%\"==\"X Y\" echo same"),
            Ok(if_node(
                false,
                true,
                Cond::Compare("\"%a%\"".into(), Op::Same, "\"X Y\"".into()),
                cmd("echo same"),
                None
            ))
        );
        assert_eq!(
            parse("if a == b echo x"),
            Ok(if_node(false, false, Cond::Compare("a".into(), Op::Same, "b".into()), cmd("echo x"), None))
        );
        assert_eq!(
            parse("if 3 gtr 2 echo x"),
            Ok(if_node(false, false, Cond::Compare("3".into(), Op::Gtr, "2".into()), cmd("echo x"), None))
        );
        assert_eq!(
            parse("if exist \"C:\\a b\\c.txt\" (echo y) else (echo n) & echo after"),
            Ok(Node::Then(
                b(if_node(
                    false,
                    false,
                    Cond::Exist("\"C:\\a b\\c.txt\"".into()),
                    Node::Block(b(cmd("echo y")), Vec::new()),
                    Some(Node::Block(b(cmd("echo n")), Vec::new()))
                )),
                b(cmd("echo after"))
            ))
        );
        assert_eq!(
            parse("if defined X (echo y) else echo n & echo m"),
            Ok(if_node(
                false,
                false,
                Cond::Defined("X".into()),
                Node::Block(b(cmd("echo y")), Vec::new()),
                Some(Node::Then(b(cmd("echo n ")), b(cmd("echo m"))))
            ))
        );
        // else after a command that is not a group is text
        assert_eq!(
            parse("if 1==1 echo a else echo b"),
            Ok(if_node(false, false, Cond::Compare("1".into(), Op::Same, "1".into()), cmd("echo a else echo b"), None))
        );
        // an if in a group ends at its line
        assert_eq!(
            parse("(if 1==2 echo a\necho b)"),
            Ok(Node::Block(
                b(Node::Then(
                    b(if_node(false, false, Cond::Compare("1".into(), Op::Same, "2".into()), cmd("echo a"), None)),
                    b(cmd("echo b"))
                )),
                Vec::new()
            ))
        );
        assert!(matches!(parse("if errorlevel x echo"), Err(Error::Syntax(_))));
        assert!(matches!(parse("if a==b"), Err(Error::Syntax(_))));
        assert!(matches!(parse("if a echo"), Err(Error::Syntax(_))));
    }
}
