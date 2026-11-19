//! The interpreter: `cmd.exe`'s life ([`Shell::main`]), the built-in commands, the search
//! for programs and batch files. Everything Windows does is behind [`Host`].
//!
//! Built-in commands (Learn: each command's page): `echo`, `set` (with `/a` and `/p`), `if`,
//! `exit` (with `/b`), `rem`, `cd`/`chdir`, `pushd`, `popd`, `type`, `md`/`mkdir`,
//! `del`/`erase`, `setlocal`, `endlocal`, `call`, `goto`, `shift`, and `cls`, `title`,
//! `color` (nothing to do without a console window of our own). Any other name is a program:
//! looked for in the current directory, then in each `PATH` directory, with each `PATHEXT`
//! extension when it has none (Learn: "Command shell overview"); a `.bat` or `.cmd` file runs
//! in this shell, any other program is started and waited for, and its exit code becomes
//! `ERRORLEVEL`.

use crate::args::{self, Mode};
use crate::expand::{self, BatchArgs, Context};
use crate::parse::{self, Cond, Node, Op, Redir, Target};
use crate::{arith, eq_ic};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// How a redirection opens its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// `<`: read.
    Read,
    /// `>`: create or truncate, write.
    Write,
    /// `>>`: create or open, write at the end.
    Append,
}

/// What a path names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
}

/// The standard handles of a command: input, output, errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Io<F> {
    pub handles: [F; 3],
    /// Whether any of them is not `cmd.exe`'s own (a program then gets them explicitly).
    pub redirected: bool,
}

/// What `cmd.exe` needs from the system.
pub trait Host {
    /// An open file or device.
    type File: Copy + PartialEq + core::fmt::Debug;
    /// `cmd.exe`'s own standard handle `n` (0 input, 1 output, 2 errors).
    fn std_handle(&self, n: u8) -> Self::File;
    /// Write text (lines end with `\r\n`).
    fn write(&mut self, f: Self::File, text: &str);
    /// One line without its line break; `None` at the end.
    fn read_line(&mut self, f: Self::File) -> Option<String>;
    fn open(&mut self, path: &str, mode: OpenMode) -> Option<Self::File>;
    fn close(&mut self, f: Self::File);
    /// An environment variable (names without case).
    fn var(&self, name: &str) -> Option<String>;
    /// Set (`Some`) or remove (`None`) an environment variable.
    fn set_var(&mut self, name: &str, value: Option<&str>);
    /// Every environment variable, as stored.
    fn vars(&self) -> Vec<(String, String)>;
    fn current_dir(&self) -> String;
    fn set_current_dir(&mut self, path: &str) -> bool;
    /// `path` made absolute against the current directory.
    fn full_path(&self, path: &str) -> String;
    fn kind(&self, path: &str) -> Option<Kind>;
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;
    fn make_dir(&mut self, path: &str) -> bool;
    fn delete_file(&mut self, path: &str) -> bool;
    /// Start `program` with `command_line` and these handles and wait for it: its exit code,
    /// or the Win32 error that kept it from starting.
    fn run(&mut self, program: &str, command_line: &str, io: &Io<Self::File>) -> Result<u32, u32>;
    fn random(&mut self) -> u32;
}

/// What stops the commands that follow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// `exit [n]`: `cmd.exe` ends.
    Exit(i32),
    /// `exit /b`, `goto :eof`, or a batch file that runs another without `call`: the running
    /// batch file ends.
    ExitBatch,
    /// `goto label`.
    Goto(String),
}

/// A command's result: whether it succeeded (for `&&` and `||`), or a [`Stop`].
pub type Step = Result<bool, Stop>;

/// `ERRORLEVEL` when a command is not a built-in and no program has its name.
pub const NOT_FOUND: i32 = 9009;

/// A running batch file.
struct Frame {
    args: BatchArgs,
    /// `locals.len()` when it started: its `setlocal`s are above.
    locals_at: usize,
}

/// What `setlocal` saved.
struct Local {
    vars: Vec<(String, String)>,
    delayed: bool,
    extensions: bool,
}

/// `cmd.exe`'s state.
pub struct Shell<H: Host> {
    pub host: H,
    errorlevel: i32,
    echo: bool,
    delayed: bool,
    extensions: bool,
    command_line: String,
    batch: Vec<Frame>,
    locals: Vec<Local>,
    dirs: Vec<String>,
}

impl<H: Host> core::fmt::Debug for Shell<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Shell")
            .field("errorlevel", &self.errorlevel)
            .field("echo", &self.echo)
            .field("delayed", &self.delayed)
            .field("extensions", &self.extensions)
            .field("batch_depth", &self.batch.len())
            .finish_non_exhaustive()
    }
}

/// A file's text: UTF-8 (a byte order mark dropped) or, failing that, one character per byte.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF_u8, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match core::str::from_utf8(bytes) {
        Ok(s) => String::from(s),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

/// The first word of `text` (a quoted part keeps its spaces) and the rest after it.
fn first_word(text: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && matches!(c, ' ' | '\t') {
            return (&text[..i], &text[i..]);
        }
    }
    (text, "")
}

/// `dir\name`.
fn join(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('"');
    if dir.ends_with(['\\', '/']) { format!("{dir}{name}") } else { format!("{dir}\\{name}") }
}

/// The built-in commands, by the name that starts a command.
const BUILTINS: &[&str] = &[
    "echo", "set", "exit", "rem", "cd", "chdir", "pushd", "popd", "type", "md", "mkdir", "del", "erase", "setlocal",
    "endlocal", "call", "goto", "shift", "cls", "title", "color",
];

/// A built-in's name at the start of `text` and the rest after it: the name is letters
/// followed by the end, white space or one of `. ( / \ , ; = + : [ ]` (`echo.`, `cd..`).
fn builtin(text: &str) -> Option<(&'static str, &str)> {
    let n = text.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(text.len());
    let rest = &text[n..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t', '.', '(', '/', '\\', ',', ';', '=', '+', ':', '[', ']']) {
        return None;
    }
    BUILTINS.iter().find(|b| eq_ic(b, &text[..n])).map(|b| (*b, rest))
}

impl<H: Host> Shell<H> {
    /// A shell for `cmd.exe` started with `command_line` (its whole command line).
    pub fn new(host: H, command_line: &str) -> Self {
        Shell {
            host,
            errorlevel: 0,
            echo: true,
            delayed: false,
            extensions: true,
            command_line: String::from(command_line),
            batch: Vec::new(),
            locals: Vec::new(),
            dirs: Vec::new(),
        }
    }

    /// `ERRORLEVEL`.
    pub fn errorlevel(&self) -> i32 {
        self.errorlevel
    }

    /// Run `cmd.exe` by its command line; the process's exit code.
    pub fn main(&mut self) -> i32 {
        let line = self.command_line.clone();
        let inv = args::parse(args::after_program_name(&line));
        self.echo = !inv.echo_off;
        self.delayed = inv.delayed;
        self.extensions = inv.extensions;
        if inv.mode != Mode::Interactive {
            let command = args::command_quotes(&inv.command, inv.strip_quotes, |s| self.find_program(s).is_some());
            if let Err(Stop::Exit(code)) = self.line(&command) {
                return code;
            }
            if inv.mode == Mode::Run {
                return self.errorlevel;
            }
        }
        self.interactive()
    }

    /// Read commands from standard input until `exit` or its end.
    fn interactive(&mut self) -> i32 {
        let (input, out) = (self.host.std_handle(0), self.host.std_handle(1));
        loop {
            if self.echo {
                let prompt = format!("\r\n{}>", self.host.current_dir());
                self.host.write(out, &prompt);
            }
            let Some(line) = self.host.read_line(input) else {
                return self.errorlevel;
            };
            if let Err(Stop::Exit(code)) = self.line(&line) {
                return code;
            }
        }
    }

    fn std_io(&self) -> Io<H::File> {
        Io { handles: [0, 1, 2].map(|n| self.host.std_handle(n)), redirected: false }
    }

    fn error(&mut self, io: &Io<H::File>, text: &str) {
        let text = format!("{text}\r\n");
        self.host.write(io.handles[2], &text);
    }

    fn print(&mut self, io: &Io<H::File>, text: &str) {
        self.host.write(io.handles[1], text);
    }

    /// Run one command line (outside a batch file, or a line of one).
    pub fn line(&mut self, text: &str) -> Result<(), Stop> {
        let expanded = self.expand(text);
        let io = self.std_io();
        match parse::parse(&expanded) {
            Ok(node) => self.exec(&node, &io).map(|_| ()),
            Err(e) => {
                self.syntax(&io, &e);
                Ok(())
            }
        }
    }

    fn syntax(&mut self, io: &Io<H::File>, e: &parse::Error) {
        let what = match e {
            parse::Error::Unclosed => "a ( is not closed",
            parse::Error::Syntax(s) => s,
        };
        self.error(io, &format!("cmd: syntax error: {what}"));
        self.errorlevel = 255;
    }

    /// A variable, the dynamic ones included (Learn: "set": `%CD%`, `%ERRORLEVEL%`,
    /// `%CMDCMDLINE%`, `%CMDEXTVERSION%`, `%RANDOM%`, when no variable has the name).
    fn lookup(&self, name: &str) -> Option<String> {
        if let Some(v) = self.host.var(name) {
            return Some(v);
        }
        if !self.extensions {
            return None;
        }
        match name.to_ascii_uppercase().as_str() {
            "CD" => Some(self.host.current_dir()),
            "ERRORLEVEL" => Some(self.errorlevel.to_string()),
            "CMDCMDLINE" => Some(self.command_line.clone()),
            "CMDEXTVERSION" => Some(String::from("2")),
            _ => None,
        }
    }

    /// `%…%` expansion of a line.
    fn expand(&mut self, text: &str) -> String {
        // %RANDOM% is the one variable whose reading changes state.
        let random =
            (self.extensions && self.host.var("RANDOM").is_none() && text.to_ascii_uppercase().contains("%RANDOM%"))
                .then(|| (self.host.random() & 0x7FFF).to_string());
        let var = |n: &str| if eq_ic(n, "RANDOM") && random.is_some() { random.clone() } else { self.lookup(n) };
        let full = |p: &str| Some(self.host.full_path(p));
        let cx = Context { batch: self.batch.last().map(|f| &f.args), var: &var, full_path: &full };
        expand::percent(text, &cx)
    }

    fn exec(&mut self, node: &Node, io: &Io<H::File>) -> Step {
        match node {
            Node::Empty => Ok(true),
            Node::Then(a, b) => {
                self.exec(a, io)?;
                self.exec(b, io)
            }
            Node::And(a, b) => {
                if self.exec(a, io)? {
                    self.exec(b, io)
                } else {
                    Ok(false)
                }
            }
            Node::Or(a, b) => {
                if self.exec(a, io)? {
                    Ok(true)
                } else {
                    self.exec(b, io)
                }
            }
            Node::Pipe(a, b) => self.pipe(a, b, io),
            Node::Block(body, redirs) => self.with_redirs(redirs, io, |s, io| s.exec(body, io)),
            Node::If(i) => {
                if self.test(&i.cond, i.ignore_case) != i.not {
                    self.exec(&i.then, io)
                } else if let Some(o) = &i.otherwise {
                    self.exec(o, io)
                } else {
                    Ok(true)
                }
            }
            Node::Command(c) => {
                let text = c.text.clone();
                self.with_redirs(&c.redirs, io, |s, io| s.command(&text, io))
            }
        }
    }

    /// Run `f` with `redirs` applied to `io`; the files it opened are closed after.
    fn with_redirs(
        &mut self,
        redirs: &[Redir],
        io: &Io<H::File>,
        f: impl FnOnce(&mut Self, &Io<H::File>) -> Step,
    ) -> Step {
        if redirs.is_empty() {
            return f(self, io);
        }
        let mut handles = io.handles;
        let mut opened = Vec::new();
        for r in redirs {
            let h = r.handle as usize;
            if h > 2 {
                continue;
            }
            let (path, mode) = match &r.to {
                Target::Handle(n) => {
                    if (*n as usize) < 3 {
                        handles[h] = handles[*n as usize];
                    }
                    continue;
                }
                Target::Read(p) => (p, OpenMode::Read),
                Target::Write(p) => (p, OpenMode::Write),
                Target::Append(p) => (p, OpenMode::Append),
            };
            match self.host.open(path, mode) {
                Some(file) => {
                    opened.push(file);
                    handles[h] = file;
                }
                None => {
                    for f in opened {
                        self.host.close(f);
                    }
                    self.error(io, &format!("cmd: cannot open {path}"));
                    self.errorlevel = 1;
                    return Ok(false);
                }
            }
        }
        let inner = Io { handles, redirected: true };
        let r = f(self, &inner);
        for f in opened {
            self.host.close(f);
        }
        r
    }

    /// `a | b`: `a` runs to its end with its output in a temporary file, then `b` reads it.
    fn pipe(&mut self, a: &Node, b: &Node, io: &Io<H::File>) -> Step {
        let dir = self.host.var("TEMP").unwrap_or_else(|| self.host.current_dir());
        let path = join(&dir, &format!("poly-cmd-pipe-{:08x}.tmp", self.host.random()));
        let Some(w) = self.host.open(&path, OpenMode::Write) else {
            self.error(io, "cmd: cannot make a pipe");
            self.errorlevel = 1;
            return Ok(false);
        };
        let first = self.exec(a, &Io { handles: [io.handles[0], w, io.handles[2]], redirected: true });
        self.host.close(w);
        let result = match first {
            Err(stop) => Err(stop),
            Ok(_) => match self.host.open(&path, OpenMode::Read) {
                Some(r) => {
                    let second = self.exec(b, &Io { handles: [r, io.handles[1], io.handles[2]], redirected: true });
                    self.host.close(r);
                    second
                }
                None => Ok(false),
            },
        };
        self.host.delete_file(&path);
        result
    }

    /// The condition of an `if`.
    fn test(&mut self, cond: &Cond, ignore_case: bool) -> bool {
        match cond {
            Cond::ErrorLevel(n) => i64::from(self.errorlevel) >= *n,
            Cond::Exist(p) => self.host.kind(p.trim_matches('"')).is_some(),
            Cond::Defined(n) => self.host.var(n).is_some(),
            Cond::CmdExtVersion(n) => 2 >= *n,
            Cond::Compare(a, op, b) => {
                let (x, y) = if ignore_case { (a.to_lowercase(), b.to_lowercase()) } else { (a.clone(), b.clone()) };
                let order = match (arith::number(a), arith::number(b)) {
                    (Some(m), Some(n)) if *op != Op::Same => m.cmp(&n),
                    _ => x.cmp(&y),
                };
                match op {
                    Op::Same | Op::Equ => order.is_eq(),
                    Op::Neq => order.is_ne(),
                    Op::Lss => order.is_lt(),
                    Op::Leq => order.is_le(),
                    Op::Gtr => order.is_gt(),
                    Op::Geq => order.is_ge(),
                }
            }
        }
    }

    /// One command: a built-in or a program.
    fn command(&mut self, text: &str, io: &Io<H::File>) -> Step {
        let text = if self.delayed {
            let var = |n: &str| self.lookup(n);
            let full = |p: &str| Some(self.host.full_path(p));
            let cx = Context { batch: self.batch.last().map(|f| &f.args), var: &var, full_path: &full };
            expand::delayed(text, &cx)
        } else {
            String::from(text)
        };
        let text = text.trim_start_matches([' ', '\t']);
        let Some((name, rest)) = builtin(text) else {
            return self.program(text, io);
        };
        match name {
            "echo" => self.echo(rest, io),
            "set" => self.set(rest, io),
            "exit" => self.exit(rest),
            "rem" | "cls" | "title" | "color" => Ok(true),
            "cd" | "chdir" => self.cd(rest, io),
            "pushd" => {
                let here = self.host.current_dir();
                let target = rest.trim().trim_matches('"').to_string();
                if target.is_empty() {
                    return Ok(true);
                }
                let ok = self.change_dir(&target, io);
                if ok {
                    self.dirs.push(here);
                }
                Ok(ok)
            }
            "popd" => match self.dirs.pop() {
                Some(d) => Ok(self.change_dir(&d, io)),
                None => Ok(true),
            },
            "type" => self.type_files(rest, io),
            "md" | "mkdir" => self.make_dirs(rest, io),
            "del" | "erase" => self.delete(rest, io),
            "setlocal" => {
                self.setlocal(rest);
                Ok(true)
            }
            "endlocal" => {
                if self.batch.last().is_some_and(|f| self.locals.len() > f.locals_at) {
                    self.endlocal();
                }
                Ok(true)
            }
            "call" => self.call(rest, io),
            "goto" => {
                let label = rest.trim_start_matches([' ', '\t', ':']);
                let label = first_word(label).0;
                if self.batch.is_empty() {
                    self.error(io, "cmd: goto works only in a batch file");
                    return Ok(false);
                }
                if eq_ic(label, "eof") { Err(Stop::ExitBatch) } else { Err(Stop::Goto(String::from(label))) }
            }
            "shift" => {
                if let Some(f) = self.batch.last_mut() {
                    f.args.shift += 1;
                }
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// `echo [on|off|message]`, `echo.` (Learn: "echo").
    fn echo(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let mut chars = rest.chars();
        let delimiter = chars.next();
        let body = chars.as_str();
        if delimiter.is_none_or(|d| matches!(d, ' ' | '\t')) {
            let word = body.trim();
            if word.is_empty() {
                let state = if self.echo { "ECHO is on.\r\n" } else { "ECHO is off.\r\n" };
                self.print(io, state);
                return Ok(true);
            }
            if eq_ic(word, "on") || eq_ic(word, "off") {
                self.echo = eq_ic(word, "on");
                return Ok(true);
            }
        }
        let text = format!("{body}\r\n");
        self.print(io, &text);
        Ok(true)
    }

    /// `set`, `set name=value`, `set name=`, `set prefix`, `set "name=value"`, `set /a`,
    /// `set /p` (Learn: "set").
    fn set(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let a = rest.strip_prefix([' ', '\t', ',', ';', '=']).unwrap_or(rest).trim_start_matches([' ', '\t']);
        let switch = |s: &str, sw: &str| {
            s.get(..2).is_some_and(|p| eq_ic(p, sw))
                && s[2..].chars().next().is_none_or(|c| matches!(c, ' ' | '\t' | '"'))
        };
        if switch(a, "/a") {
            return self.set_arith(&a[2..], io);
        }
        if switch(a, "/p") {
            return self.set_prompt(a[2..].trim_start(), io);
        }
        let a = match a.strip_prefix('"') {
            Some(inner) => inner.rfind('"').map_or(inner, |i| &inner[..i]),
            None => a,
        };
        match a.split_once('=') {
            Some(("", _)) => {
                self.error(io, "cmd: set needs a name before =");
                self.errorlevel = 1;
                Ok(false)
            }
            Some((name, value)) => {
                self.host.set_var(name, if value.is_empty() { None } else { Some(value) });
                Ok(true)
            }
            None => {
                let mut vars: Vec<(String, String)> = self
                    .host
                    .vars()
                    .into_iter()
                    .filter(|(n, _)| !n.starts_with('=') && n.len() >= a.len() && eq_ic(&n[..a.len()], a))
                    .collect();
                if vars.is_empty() {
                    self.error(io, &format!("cmd: environment variable {a} is not defined"));
                    self.errorlevel = 1;
                    return Ok(false);
                }
                vars.sort_by_key(|(n, _)| n.to_ascii_uppercase());
                let text: String = vars.iter().map(|(n, v)| format!("{n}={v}\r\n")).collect();
                self.print(io, &text);
                Ok(true)
            }
        }
    }

    /// `set /a expression`: on the command line the value is shown (without a line break).
    fn set_arith(&mut self, expr: &str, io: &Io<H::File>) -> Step {
        struct V<'a, H: Host>(&'a mut H);
        impl<H: Host> arith::Vars for V<'_, H> {
            fn get(&self, name: &str) -> Option<String> {
                self.0.var(name)
            }
            fn set(&mut self, name: &str, value: &str) {
                self.0.set_var(name, Some(value));
            }
        }
        match arith::eval(expr, &mut V(&mut self.host)) {
            Ok(v) => {
                if self.batch.is_empty() {
                    self.print(io, &v.to_string());
                }
                Ok(true)
            }
            Err(e) => {
                let what = match e {
                    arith::Error::DivideByZero => "cmd: set /a: division by zero",
                    arith::Error::Syntax => "cmd: set /a: not an expression",
                };
                self.error(io, what);
                self.errorlevel = 1;
                Ok(false)
            }
        }
    }

    /// `set /p name=[prompt]`: the variable gets a line of input.
    fn set_prompt(&mut self, a: &str, io: &Io<H::File>) -> Step {
        let a = match a.strip_prefix('"') {
            Some(inner) => inner.rfind('"').map_or(inner, |i| &inner[..i]),
            None => a,
        };
        let Some((name, prompt)) = a.split_once('=') else {
            self.error(io, "cmd: set /p needs name=");
            self.errorlevel = 1;
            return Ok(false);
        };
        self.print(io, prompt);
        match self.host.read_line(io.handles[0]) {
            Some(line) if !line.is_empty() => {
                self.host.set_var(name, Some(&line));
                Ok(true)
            }
            _ => {
                self.errorlevel = 1;
                Ok(false)
            }
        }
    }

    /// `exit [/b] [code]` (Learn: "exit"): `/b` in a batch file ends it and sets
    /// `ERRORLEVEL`; otherwise `cmd.exe` ends with the code (with no code: `ERRORLEVEL`).
    fn exit(&mut self, rest: &str) -> Step {
        let a = rest.trim_start_matches([' ', '\t']);
        let (b, a) = match a.get(..2) {
            Some(s) if eq_ic(s, "/b") => (true, a[2..].trim_start()),
            _ => (false, a),
        };
        let word = first_word(a).0;
        let code = if word.is_empty() { None } else { Some(arith::number(word).map_or(0, |n| n as i32)) };
        if b && !self.batch.is_empty() {
            if let Some(c) = code {
                self.errorlevel = c;
            }
            return Err(Stop::ExitBatch);
        }
        Err(Stop::Exit(code.unwrap_or(self.errorlevel)))
    }

    fn change_dir(&mut self, path: &str, io: &Io<H::File>) -> bool {
        if self.host.set_current_dir(path) {
            return true;
        }
        self.error(io, &format!("cmd: cannot change to {path}"));
        self.errorlevel = 1;
        false
    }

    /// `cd [/d] [path]`: no path shows the current directory.
    fn cd(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let a = rest.trim();
        let a = match a.get(..2) {
            Some(s) if eq_ic(s, "/d") => a[2..].trim_start(),
            _ => a,
        };
        let path = a.trim_matches('"');
        if path.is_empty() {
            let text = format!("{}\r\n", self.host.current_dir());
            self.print(io, &text);
            return Ok(true);
        }
        Ok(self.change_dir(path, io))
    }

    /// The file arguments of a command (switches left out, quotes removed).
    fn file_args(rest: &str) -> Vec<String> {
        expand::split_args(rest)
            .into_iter()
            .filter(|a| !a.starts_with('/'))
            .map(|a| a.trim_matches('"').to_string())
            .collect()
    }

    /// `type file…`.
    fn type_files(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let mut ok = true;
        for f in Self::file_args(rest) {
            match self.host.read_file(&f) {
                Some(bytes) => {
                    let text = decode(&bytes);
                    self.print(io, &text);
                }
                None => {
                    self.error(io, &format!("cmd: cannot read {f}"));
                    self.errorlevel = 1;
                    ok = false;
                }
            }
        }
        Ok(ok)
    }

    /// `md dir…`, making the directories on the way (command extensions).
    fn make_dirs(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let mut ok = true;
        for d in Self::file_args(rest) {
            let full = self.host.full_path(&d);
            if self.host.kind(&full).is_some() {
                self.error(io, &format!("cmd: {d} exists already"));
                self.errorlevel = 1;
                ok = false;
                continue;
            }
            let mut at = 0;
            let mut made = true;
            while made {
                let end = full[at..].find(['\\', '/']).map_or(full.len(), |i| at + i);
                let part = &full[..end];
                // the drive and the root exist
                if end > 3 && self.host.kind(part).is_none() {
                    made = self.host.make_dir(part);
                }
                if end == full.len() {
                    break;
                }
                at = end + 1;
            }
            if !made {
                self.error(io, &format!("cmd: cannot make {d}"));
                self.errorlevel = 1;
                ok = false;
            }
        }
        Ok(ok)
    }

    /// `del file…` (no wildcards).
    fn delete(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        for f in Self::file_args(rest) {
            if f.contains(['*', '?']) {
                self.error(io, &format!("cmd: del: wildcards are not supported ({f})"));
                continue;
            }
            if !self.host.delete_file(&f) {
                self.error(io, &format!("cmd: cannot delete {f}"));
            }
        }
        Ok(true)
    }

    /// `setlocal [enabledelayedexpansion|disabledelayedexpansion|enableextensions|disableextensions]`
    /// (Learn: "setlocal": no effect outside a batch file).
    fn setlocal(&mut self, rest: &str) {
        if self.batch.is_empty() {
            return;
        }
        self.locals.push(Local { vars: self.host.vars(), delayed: self.delayed, extensions: self.extensions });
        for w in expand::split_args(rest) {
            match w.to_ascii_lowercase().as_str() {
                "enabledelayedexpansion" => self.delayed = true,
                "disabledelayedexpansion" => self.delayed = false,
                "enableextensions" => self.extensions = true,
                "disableextensions" => self.extensions = false,
                _ => {}
            }
        }
    }

    /// Undo the last `setlocal`.
    fn endlocal(&mut self) {
        let Some(l) = self.locals.pop() else { return };
        for (name, _) in self.host.vars() {
            if !name.starts_with('=') && !l.vars.iter().any(|(n, _)| eq_ic(n, &name)) {
                self.host.set_var(&name, None);
            }
        }
        for (name, value) in &l.vars {
            if self.host.var(name).as_deref() != Some(value.as_str()) {
                self.host.set_var(name, Some(value));
            }
        }
        self.delayed = l.delayed;
        self.extensions = l.extensions;
    }

    /// `call file.bat [args]`, `call :label [args]`, `call command`.
    fn call(&mut self, rest: &str, io: &Io<H::File>) -> Step {
        let a = rest.trim_start_matches([' ', '\t']);
        if let Some(label) = a.strip_prefix(':') {
            let Some(path) = self.batch.last().map(|f| f.args.full.clone()) else {
                self.error(io, "cmd: call :label works only in a batch file");
                return Ok(false);
            };
            let (name, args) = first_word(label);
            let zero = format!(":{name}");
            return self.run_batch(&path, &zero, args, Some(name), io);
        }
        if a.is_empty() {
            return Ok(true);
        }
        match parse::parse(a) {
            // a batch file started by call returns here
            Ok(node) => self.exec_called(&node, io),
            Err(e) => {
                self.syntax(io, &e);
                Ok(false)
            }
        }
    }

    /// A called command: a batch file named by it runs and returns.
    fn exec_called(&mut self, node: &Node, io: &Io<H::File>) -> Step {
        match node {
            Node::Command(c) if c.redirs.is_empty() => {
                let text = c.text.trim_start();
                if builtin(text).is_none() {
                    let (name, args) = first_word(text);
                    if let Some(path) = self.find_program(&name.replace('"', ""))
                        && Self::is_batch(&path)
                    {
                        return self.run_batch(&path, name, args, None, io);
                    }
                }
                self.exec(node, io)
            }
            _ => self.exec(node, io),
        }
    }

    fn is_batch(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with(".bat") || lower.ends_with(".cmd")
    }

    /// Where the program `name` is: as given when it has a directory, else in the current
    /// directory, then in each `PATH` directory; with each `PATHEXT` extension (and as it is
    /// when it has an extension).
    pub fn find_program(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains(['*', '?', '<', '>', '|']) {
            return None;
        }
        let pathext = self.host.var("PATHEXT").unwrap_or_else(|| String::from(".COM;.EXE;.BAT;.CMD"));
        let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
        let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let has_ext = file.contains('.');
        let dirs: Vec<String> = if name.contains(['\\', '/', ':']) {
            vec![String::new()]
        } else {
            let mut d = vec![self.host.current_dir()];
            if let Some(path) = self.host.var("PATH") {
                d.extend(path.split(';').map(|p| p.trim_matches('"')).filter(|p| !p.is_empty()).map(String::from));
            }
            d
        };
        for dir in dirs {
            let base = if dir.is_empty() { String::from(name) } else { join(&dir, name) };
            let candidates = has_ext.then(|| base.clone()).into_iter().chain(exts.iter().map(|e| format!("{base}{e}")));
            for c in candidates {
                if self.host.kind(&c) == Some(Kind::File) {
                    return Some(self.host.full_path(&c));
                }
            }
        }
        None
    }

    /// A command that is no built-in: a drive change, a batch file or a program.
    fn program(&mut self, text: &str, io: &Io<H::File>) -> Step {
        let (word, args) = first_word(text);
        let name = word.replace('"', "");
        // `d:` makes another drive current
        if name.len() == 2 && name.ends_with(':') && args.trim().is_empty() {
            let target = self.host.full_path(&name);
            return Ok(self.change_dir(&target, io));
        }
        let Some(path) = self.find_program(&name) else {
            self.error(io, &format!("cmd: '{name}' is not a built-in command, a program or a batch file"));
            self.errorlevel = NOT_FOUND;
            return Ok(false);
        };
        if Self::is_batch(&path) {
            // without `call`, the batch file that runs it does not go on
            let nested = !self.batch.is_empty();
            let r = self.run_batch(&path, word, args, None, io)?;
            return if nested { Err(Stop::ExitBatch) } else { Ok(r) };
        }
        let command_line = text.trim_end_matches(['\r', '\n']);
        match self.host.run(&path, command_line, io) {
            Ok(code) => {
                self.errorlevel = code as i32;
                Ok(code == 0)
            }
            Err(e) => {
                self.error(io, &format!("cmd: cannot start {path} (error {e})"));
                self.errorlevel = e as i32;
                Ok(false)
            }
        }
    }

    /// Run the batch file `path` (named `zero`) with `args`, from its start or from the line
    /// after `label`.
    fn run_batch(&mut self, path: &str, zero: &str, args: &str, label: Option<&str>, io: &Io<H::File>) -> Step {
        let Some(bytes) = self.host.read_file(path) else {
            self.error(io, &format!("cmd: cannot read {path}"));
            self.errorlevel = 1;
            return Ok(false);
        };
        let text = decode(&bytes);
        let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        let mut pc = match label {
            None => 0,
            Some(l) => match find_label(&lines, l) {
                Some(k) => k,
                None => {
                    self.error(io, &format!("cmd: no label {l}"));
                    self.errorlevel = 1;
                    return Ok(false);
                }
            },
        };
        let locals_at = self.locals.len();
        let args =
            BatchArgs { zero: String::from(zero), full: String::from(path), args: expand::split_args(args), shift: 0 };
        self.batch.push(Frame { args, locals_at });
        let result = loop {
            let Some(raw) = lines.get(pc) else { break Ok(self.errorlevel == 0) };
            pc += 1;
            let t = raw.trim_start_matches([' ', '\t']);
            if t.is_empty() || t.starts_with(':') {
                continue;
            }
            let mut expanded = self.expand(raw);
            let node = loop {
                match parse::parse(&expanded) {
                    Ok(n) => break Some(n),
                    Err(parse::Error::Unclosed) if pc < lines.len() => {
                        let next = self.expand(lines[pc]);
                        pc += 1;
                        expanded.push('\n');
                        expanded.push_str(&next);
                    }
                    Err(e) => {
                        self.syntax(io, &e);
                        break None;
                    }
                }
            };
            let Some(node) = node else { continue };
            if self.echo && !t.starts_with('@') {
                let echoed = format!("\r\n{}>{}\r\n", self.host.current_dir(), expanded.trim());
                self.print(io, &echoed);
            }
            match self.exec(&node, io) {
                Ok(_) => {}
                Err(Stop::ExitBatch) => break Ok(self.errorlevel == 0),
                Err(Stop::Goto(l)) => match find_label(&lines, &l) {
                    Some(k) => pc = k,
                    None => {
                        self.error(io, &format!("cmd: no label {l}"));
                        self.errorlevel = 1;
                        break Ok(false);
                    }
                },
                Err(stop) => break Err(stop),
            }
        };
        while self.locals.len() > locals_at {
            self.endlocal();
        }
        self.batch.pop();
        result
    }
}

/// The index of the line after `:label`.
fn find_label(lines: &[&str], label: &str) -> Option<usize> {
    lines
        .iter()
        .position(|l| {
            l.trim_start().strip_prefix(':').is_some_and(|rest| eq_ic(first_word(rest.trim_start()).0, label))
        })
        .map(|k| k + 1)
}

#[cfg(test)]
mod tests;
