//! The interpreter against a host that keeps files, variables and programs in memory.

use super::*;
use std::collections::{BTreeMap, VecDeque};

/// A program of the mock: gets its command line, the host and its handles; returns its exit
/// code.
type Program = fn(&str, &mut Mock, &Io<usize>) -> u32;

struct Mock {
    /// Files by their full path in lower case; `<out>` and `<err>` are the standard output
    /// and errors.
    files: BTreeMap<String, Vec<u8>>,
    dirs: Vec<String>,
    env: Vec<(String, String)>,
    cwd: String,
    input: VecDeque<String>,
    /// Open handles: the file and the read position (0..=2 are the standard ones).
    handles: Vec<(String, usize)>,
    programs: BTreeMap<String, Program>,
    /// The programs started: (path, command line).
    runs: Vec<(String, String)>,
    seed: u32,
}

/// `prog.exe [code] …`: says how it was started; exits with the number after its name.
fn prog(cmdline: &str, m: &mut Mock, io: &Io<usize>) -> u32 {
    m.write(io.handles[1], &format!("prog ran: [{cmdline}]\r\n"));
    let code = first_word(cmdline).1.split_whitespace().next().and_then(|w| w.parse().ok()).unwrap_or(0);
    if code == 42 {
        m.write(io.handles[2], "prog error\r\n");
    }
    code
}

/// `upper.exe`: copies its input in capitals.
fn upper(_: &str, m: &mut Mock, io: &Io<usize>) -> u32 {
    while let Some(l) = m.read_line(io.handles[0]) {
        m.write(io.handles[1], &format!("{}\r\n", l.to_uppercase()));
    }
    0
}

/// `envcode.exe`: exits with the value of `CODE` from its environment.
fn envcode(_: &str, m: &mut Mock, _: &Io<usize>) -> u32 {
    m.var("CODE").and_then(|v| v.parse().ok()).unwrap_or(99)
}

fn mock() -> Mock {
    let mut m = Mock {
        files: BTreeMap::new(),
        dirs: ["c:\\", "c:\\work", "c:\\windows", "c:\\windows\\system32", "c:\\temp", "c:\\program files", "d:\\"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        env: vec![
            ("Path".into(), "C:\\Windows\\System32;\"C:\\Program Files\"".into()),
            ("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()),
            ("TEMP".into(), "C:\\temp".into()),
        ],
        cwd: "C:\\work".into(),
        input: VecDeque::new(),
        handles: vec![("<in>".into(), 0), ("<out>".into(), 0), ("<err>".into(), 0)],
        programs: BTreeMap::new(),
        runs: Vec::new(),
        seed: 1,
    };
    m.program("C:\\Windows\\System32\\cmd.exe", prog);
    m.program("C:\\Windows\\System32\\prog.exe", prog);
    m.program("C:\\Windows\\System32\\upper.exe", upper);
    m.program("C:\\Windows\\System32\\envcode.exe", envcode);
    m.program("C:\\Program Files\\app.exe", prog);
    m
}

impl Mock {
    fn program(&mut self, path: &str, f: Program) {
        self.files.insert(path.to_lowercase(), b"MZ".to_vec());
        self.programs.insert(path.to_lowercase(), f);
    }

    fn file(&mut self, path: &str, text: &str) {
        let full = self.full_path(path).to_lowercase();
        self.files.insert(full, text.as_bytes().to_vec());
    }

    fn text(&self, key: &str) -> String {
        String::from_utf8(self.files.get(key).cloned().unwrap_or_default()).unwrap()
    }
}

impl Host for Mock {
    type File = usize;

    fn std_handle(&self, n: u8) -> usize {
        n as usize
    }

    fn write(&mut self, f: usize, text: &str) {
        let key = self.handles[f].0.clone();
        if key != "nul" {
            self.files.entry(key).or_default().extend_from_slice(text.as_bytes());
        }
    }

    fn read_line(&mut self, f: usize) -> Option<String> {
        if f == 0 {
            return self.input.pop_front();
        }
        let (key, pos) = self.handles[f].clone();
        let data = self.files.get(&key)?;
        if pos >= data.len() {
            return None;
        }
        let end = data[pos..].iter().position(|&b| b == b'\n').map_or(data.len(), |i| pos + i);
        let line = String::from_utf8(data[pos..end].to_vec()).unwrap();
        self.handles[f].1 = end + 1;
        Some(line.trim_end_matches('\r').to_string())
    }

    fn open(&mut self, path: &str, mode: OpenMode) -> Option<usize> {
        let key = if eq_ic(path, "nul") { "nul".to_string() } else { self.full_path(path).to_lowercase() };
        if key != "nul" {
            let parent = &key[..key.rfind('\\')?.max(3)];
            if !self.dirs.iter().any(|d| d == parent) {
                return None;
            }
            match mode {
                OpenMode::Read if !self.files.contains_key(&key) => return None,
                OpenMode::Read => {}
                OpenMode::Write => {
                    self.files.insert(key.clone(), Vec::new());
                }
                OpenMode::Append => {
                    self.files.entry(key.clone()).or_default();
                }
            }
        }
        self.handles.push((key, 0));
        Some(self.handles.len() - 1)
    }

    fn close(&mut self, f: usize) {
        assert!(f > 2, "a standard handle is never closed");
        self.handles[f].0 = "<closed>".into();
    }

    fn var(&self, name: &str) -> Option<String> {
        self.env.iter().find(|(n, _)| eq_ic(n, name)).map(|(_, v)| v.clone())
    }

    fn set_var(&mut self, name: &str, value: Option<&str>) {
        let at = self.env.iter().position(|(n, _)| eq_ic(n, name));
        match (at, value) {
            (Some(i), Some(v)) => self.env[i].1 = v.into(),
            (None, Some(v)) => self.env.push((name.into(), v.into())),
            (Some(i), None) => {
                self.env.remove(i);
            }
            (None, None) => {}
        }
    }

    fn vars(&self) -> Vec<(String, String)> {
        self.env.clone()
    }

    fn current_dir(&self) -> String {
        self.cwd.clone()
    }

    fn set_current_dir(&mut self, path: &str) -> bool {
        let full = self.full_path(path);
        if self.kind(&full) == Some(Kind::Dir) {
            self.cwd = full.trim_end_matches('\\').to_string();
            if self.cwd.len() == 2 {
                self.cwd.push('\\');
            }
            true
        } else {
            false
        }
    }

    fn full_path(&self, path: &str) -> String {
        let p = path.replace('/', "\\");
        let joined = if p.len() == 2 && p.ends_with(':') {
            format!("{}\\", p.to_uppercase())
        } else if p.as_bytes().get(1) == Some(&b':') {
            p
        } else if p.starts_with('\\') {
            format!("{}{p}", &self.cwd[..2])
        } else {
            join(&self.cwd, &p)
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in joined.split('\\') {
            match part {
                "." | "" => {}
                ".." => {
                    if parts.len() > 1 {
                        parts.pop();
                    }
                }
                _ => parts.push(part),
            }
        }
        let s = parts.join("\\");
        if parts.len() == 1 { format!("{s}\\") } else { s }
    }

    fn kind(&self, path: &str) -> Option<Kind> {
        let full = self.full_path(path).to_lowercase();
        if self.files.contains_key(&full) {
            Some(Kind::File)
        } else if self.dirs.contains(&full) {
            Some(Kind::Dir)
        } else {
            None
        }
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&self.full_path(path).to_lowercase()).cloned()
    }

    fn make_dir(&mut self, path: &str) -> bool {
        self.dirs.push(self.full_path(path).to_lowercase());
        true
    }

    fn delete_file(&mut self, path: &str) -> bool {
        self.files.remove(&self.full_path(path).to_lowercase()).is_some()
    }

    fn run(&mut self, program: &str, command_line: &str, io: &Io<usize>) -> Result<u32, u32> {
        self.runs.push((program.into(), command_line.into()));
        let f = *self.programs.get(&program.to_lowercase()).ok_or(2u32)?;
        Ok(f(command_line, self, io))
    }

    fn random(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        self.seed >> 8
    }
}

/// Run `cmd.exe` with `cmdline`: (exit code, output, errors, the host after).
fn cmd_with(m: Mock, cmdline: &str) -> (i32, String, String, Mock) {
    let mut sh = Shell::new(m, cmdline);
    let code = sh.main();
    let m = sh.host;
    (code, m.text("<out>"), m.text("<err>"), m)
}

fn cmd(cmdline: &str) -> (i32, String, String) {
    let (c, o, e, _) = cmd_with(mock(), cmdline);
    (c, o, e)
}

#[test]
fn exit_codes_the_tests_need() {
    // CreateProcessElevation, K32QueryWorkingSetEx, SetProcessWorkingSetSize, WTSGetActiveConsoleSessionId
    assert_eq!(cmd("C:\\Windows\\System32\\cmd.exe /c exit 3"), (3, String::new(), String::new()));
    assert_eq!(cmd("C:\\Windows\\System32\\cmd.exe /c exit").0, 0);
    // CreateProcessAsUserW's names: "cmd.exe /c exit 4", "cmd /c exit 5"
    assert_eq!(cmd("cmd.exe /c exit 4").0, 4);
    assert_eq!(cmd("cmd /c exit 5").0, 5);
    assert_eq!(cmd("cmd /C \"exit 6\"").0, 6);
    assert_eq!(cmd("cmd /c exit /b 7").0, 7);
    assert_eq!(cmd("cmd /c exit 0x10").0, 16);
    assert_eq!(cmd("cmd /c exit -1").0, -1);
    assert_eq!(cmd("cmd /c exit \"%1\"").0, 0);
    // and with a variable of the environment (CreateProcessAsUserW's "environment")
    let mut m = mock();
    m.env.push(("CODE".into(), "9".into()));
    assert_eq!(cmd_with(m, "cmd /c exit %CODE%").0, 9);
}

#[test]
fn echo() {
    assert_eq!(cmd("cmd /c echo hello  world").1, "hello  world\r\n");
    assert_eq!(cmd("cmd /c echo.").1, "\r\n");
    assert_eq!(cmd("cmd /c echo(x").1, "x\r\n");
    assert_eq!(cmd("cmd /c echo").1, "ECHO is on.\r\n");
    assert_eq!(cmd("cmd /c echo off & echo").1, "ECHO is off.\r\n");
    assert_eq!(cmd("cmd /q /c echo").1, "ECHO is off.\r\n");
    assert_eq!(cmd("cmd /c echo a & echo b&&echo c").1, "a \r\nb\r\nc\r\n");
    assert_eq!(cmd("cmd /c echo a^&b \"c & d\"").1, "a&b \"c & d\"\r\n");
}

#[test]
fn variables() {
    // %X% is expanded before the line runs
    assert_eq!(cmd("cmd /c set X=1 & echo [%X%]").1, "[%X%]\r\n");
    // delayed expansion sees it
    assert_eq!(cmd("cmd /v:on /c set X=1& echo [!X!]").1, "[1]\r\n");
    let (code, out, _, m) = cmd_with(mock(), "cmd /c set \"Greeting=hi there\" & set GREET");
    assert_eq!((code, out.as_str()), (0, "Greeting=hi there\r\n"));
    assert_eq!(m.var("greeting").as_deref(), Some("hi there"));
    let (code, _, err, m) = cmd_with(mock(), "cmd /c set TEMP= & set NOPE_");
    assert_eq!(code, 1);
    assert!(err.contains("NOPE_"));
    assert_eq!(m.var("TEMP").as_deref(), Some(" "));
    let (_, _, _, m) = cmd_with(mock(), "cmd /c set TEMP=");
    assert_eq!(m.var("TEMP"), None);
    assert_eq!(cmd("cmd /c echo %PATH:~0,10% %TEMP:temp=x%").1, "C:\\Windows C:\\x\r\n");
    assert_eq!(cmd("cmd /c echo %ERRORLEVEL% %CD% %CMDEXTVERSION%").1, "0 C:\\work 2\r\n");
    assert_eq!(cmd("cmd /c echo %CMDCMDLINE%").1, "cmd /c echo %CMDCMDLINE%\r\n");
    assert_eq!(cmd("cmd /e:off /c echo %CD%").1, "%CD%\r\n");
    let r = cmd("cmd /c echo %RANDOM%").1;
    assert!(r.trim().parse::<u32>().is_ok_and(|n| n < 32768), "{r}");
    // set /a shows its value on the command line
    assert_eq!(cmd("cmd /c set /a 6*7").1, "42");
    let (_, out, _, m) = cmd_with(mock(), "cmd /c set /a \"N=1+2\" >nul & set /a N+=1");
    assert_eq!(out, "4");
    assert_eq!(m.var("N").as_deref(), Some("4"));
    let (code, _, err) = cmd("cmd /c set /a 1/0");
    assert_eq!(code, 1);
    assert!(!err.is_empty());
    // set /p reads a line
    let mut m = mock();
    m.input.push_back("typed".into());
    let (_, out, _, m) = cmd_with(m, "cmd /c set /p ANSWER=Name? ");
    assert_eq!(out, "Name? ");
    assert_eq!(m.var("ANSWER").as_deref(), Some("typed"));
}

#[test]
fn programs_and_exit_codes() {
    let (code, out, _, m) = cmd_with(mock(), "cmd /c prog 7 \"a b\"");
    assert_eq!(code, 7);
    assert_eq!(out, "prog ran: [prog 7 \"a b\"]\r\n");
    // the extension as PATHEXT spells it
    assert_eq!(m.runs, vec![("C:\\Windows\\System32\\prog.EXE".to_string(), "prog 7 \"a b\"".to_string())]);
    // a full path, and the quote rule keeping a quoted program with spaces
    let (code, _, _, m) = cmd_with(mock(), "cmd /c \"C:\\Program Files\\app.exe\" 3");
    assert_eq!(code, 3);
    assert_eq!(m.runs[0].1, "\"C:\\Program Files\\app.exe\" 3");
    // the outer quotes removed when there are more than two
    let (code, _, _, m) = cmd_with(mock(), "cmd /c \"\"C:\\Program Files\\app.exe\" 5 \"x y\"\"");
    assert_eq!(code, 5);
    assert_eq!(m.runs[0], ("C:\\Program Files\\app.exe".into(), "\"C:\\Program Files\\app.exe\" 5 \"x y\"".into()));
    // found in a PATH directory written in quotes
    assert_eq!(cmd("cmd /c app 2").0, 2);
    // the current directory comes first
    let mut m = mock();
    m.program("C:\\work\\prog.exe", envcode);
    assert_eq!(cmd_with(m, "cmd /c prog").0, 99);
    // && and || look at the exit code
    assert_eq!(cmd("cmd /c prog 1 && echo yes || echo no").1, "prog ran: [prog 1 ]\r\nno\r\n");
    assert_eq!(cmd("cmd /c prog 0 && echo yes || echo no").1, "prog ran: [prog 0 ]\r\nyes \r\n");
    // the last program's code is cmd's
    assert_eq!(cmd("cmd /c prog 4 & echo done").0, 4);
    // not found: 9009
    let (code, _, err) = cmd("cmd /c nosuchprogram 1");
    assert_eq!(code, NOT_FOUND);
    assert!(err.contains("nosuchprogram"));
    // the child sees the environment cmd changed
    assert_eq!(cmd("cmd /c set CODE=12& envcode").0, 12);
}

#[test]
fn if_command() {
    assert_eq!(cmd("cmd /c prog 7 >nul & if errorlevel 7 echo ge7").1, "ge7\r\n");
    assert_eq!(cmd("cmd /c prog 6 >nul & if errorlevel 7 echo ge7").1, "");
    assert_eq!(cmd("cmd /c prog 6 >nul & if not errorlevel 7 echo lt7").1, "lt7\r\n");
    assert_eq!(cmd("cmd /c if \"%TEMP%\"==\"C:\\temp\" (echo same) else (echo other)").1, "same\r\n");
    assert_eq!(cmd("cmd /c if abc==ABC (echo same) else echo other").1, "other\r\n");
    assert_eq!(cmd("cmd /c if /i abc==ABC echo same").1, "same\r\n");
    assert_eq!(cmd("cmd /c if 10 gtr 9 echo num").1, "num\r\n");
    assert_eq!(cmd("cmd /c if 10 lss 9 (echo a) else echo b").1, "b\r\n");
    assert_eq!(cmd("cmd /c if exist C:\\Windows\\System32\\prog.exe echo yes").1, "yes\r\n");
    assert_eq!(cmd("cmd /c if not exist C:\\nope.txt echo no").1, "no\r\n");
    assert_eq!(cmd("cmd /c if defined TEMP echo d & echo e").1, "d \r\ne\r\n");
    assert_eq!(cmd("cmd /c if defined NOPE echo d & echo e").1, "");
    assert_eq!(cmd("cmd /c if cmdextversion 2 echo ext").1, "ext\r\n");
    let (code, _, err) = cmd("cmd /c if a echo");
    assert_eq!(code, 255);
    assert!(err.contains("syntax"));
    // exit inside a branch
    assert_eq!(cmd("cmd /c prog 3 >nul & if errorlevel 1 exit 1").0, 1);
}

#[test]
fn redirection() {
    let (_, out, _, m) = cmd_with(mock(), "cmd /c echo one> out.txt& echo two>>out.txt& type out.txt");
    assert_eq!(out, "one\r\ntwo\r\n");
    assert_eq!(m.text("c:\\work\\out.txt"), "one\r\ntwo\r\n");
    // a program's output and errors
    let (code, out, err, m) = cmd_with(mock(), "cmd /c prog 42 > C:\\temp\\log.txt 2>&1");
    assert_eq!((code, out.as_str(), err.as_str()), (42, "", ""));
    assert_eq!(m.text("c:\\temp\\log.txt"), "prog ran: [prog 42 ]\r\nprog error\r\n");
    let (_, out, err) = cmd("cmd /c prog 42 2>nul");
    assert_eq!((out.as_str(), err.as_str()), ("prog ran: [prog 42 ]\r\n", ""));
    // input
    let mut m = mock();
    m.file("C:\\work\\in.txt", "abc\r\ndef\r\n");
    let (_, out, _, _) = cmd_with(m, "cmd /c upper < in.txt");
    assert_eq!(out, "ABC\r\nDEF\r\n");
    // a group
    let (_, out, _, m) = cmd_with(mock(), "cmd /c (echo a & echo b) > g.txt & type g.txt");
    assert_eq!(out, "a \r\nb\r\n");
    assert_eq!(m.text("c:\\work\\g.txt"), "a \r\nb\r\n");
    // a file that cannot be opened
    let (code, _, err) = cmd("cmd /c echo x > C:\\nodir\\f.txt");
    assert_eq!(code, 1);
    assert!(err.contains("C:\\nodir\\f.txt"));
    // a pipe
    let (_, out, _, m) = cmd_with(mock(), "cmd /c echo hello| upper");
    assert_eq!(out, "HELLO\r\n");
    assert!(m.files.keys().all(|k| !k.contains("poly-cmd-pipe")), "the pipe's file is deleted");
}

#[test]
fn directories_and_files() {
    let (_, out, _, m) = cmd_with(mock(), "cmd /c cd .. & cd & cd /d D:\\ & cd");
    assert_eq!(out, "C:\\\r\nD:\\\r\n");
    assert_eq!(m.cwd, "D:\\");
    let (_, out, _) = cmd("cmd /c pushd C:\\temp & cd & popd & cd");
    assert_eq!(out, "C:\\temp\r\nC:\\work\r\n");
    let (code, _, err) = cmd("cmd /c cd C:\\nope");
    assert_eq!(code, 1);
    assert!(err.contains("C:\\nope"));
    let (_, _, _, m) = cmd_with(mock(), "cmd /c md a\\b\\c & echo x> a\\b\\c\\f.txt & del a\\b\\c\\f.txt");
    assert!(m.dirs.contains(&"c:\\work\\a\\b\\c".to_string()));
    assert!(!m.files.contains_key("c:\\work\\a\\b\\c\\f.txt"));
    assert_eq!(cmd("cmd /c d: & cd").1, "D:\\\r\n");
}

#[test]
fn batch_files() {
    let mut m = mock();
    m.file(
        "C:\\work\\run.bat",
        "@echo off\r\n\
         rem a comment\r\n\
         :: another\r\n\
         echo args %1 [%~2] all=%*\r\n\
         echo here %~dp0 name %~nx0\r\n\
         set COUNT=0\r\n\
         :loop\r\n\
         set /a COUNT+=1\r\n\
         if %COUNT% lss 3 goto loop\r\n\
         echo count %COUNT% 100%%\r\n\
         call :sub one two\r\n\
         echo after sub %ERRORLEVEL%\r\n\
         if %COUNT%==3 (\r\n\
           echo in block\r\n\
           echo still\r\n\
         ) else (\r\n\
           echo wrong\r\n\
         )\r\n\
         exit /b 5\r\n\
         echo not reached\r\n\
         :sub\r\n\
         echo sub %0 %1 %2\r\n\
         exit /b 2\r\n",
    );
    let (code, out, err, _) = cmd_with(m, "cmd /c run.bat first \"second arg\"");
    assert_eq!(err, "");
    assert_eq!(
        out,
        "args first [second arg] all=first \"second arg\"\r\n\
         here C:\\work\\ name run.bat\r\n\
         count 3 100%\r\n\
         sub :sub one two\r\n\
         after sub 2\r\n\
         in block\r\n\
         still\r\n"
    );
    assert_eq!(code, 5);
}

#[test]
fn batch_echo_setlocal_and_nesting() {
    let mut m = mock();
    m.file("C:\\work\\inner.cmd", "@set FROM_INNER=1\r\n@exit /b 3\r\n");
    m.file(
        "C:\\work\\outer.bat",
        "echo on-line\r\n\
         @setlocal enabledelayedexpansion\r\n\
         @set LOCAL=yes\r\n\
         @set X=1& echo !X!\r\n\
         @endlocal\r\n\
         @echo [%LOCAL%]\r\n\
         @call inner.cmd\r\n\
         @echo inner gave %ERRORLEVEL% %FROM_INNER%\r\n\
         @inner.cmd\r\n\
         @echo not reached\r\n",
    );
    let (code, out, _, m) = cmd_with(m, "cmd /c outer");
    assert_eq!(out, "\r\nC:\\work>echo on-line\r\non-line\r\n1\r\n[]\r\ninner gave 3 1\r\n");
    assert_eq!(code, 3);
    assert_eq!(m.var("LOCAL"), None);
    // setlocal outside a batch file does nothing; goto there fails
    let (_, out, err) = cmd("cmd /c setlocal & set Z=1& goto x");
    assert_eq!(out, "");
    assert!(err.contains("goto"));
}

#[test]
fn shift_and_missing_labels() {
    let mut m = mock();
    m.file("C:\\work\\s.bat", "@echo %1\r\n@shift\r\n@echo %1 %0\r\n@goto nowhere\r\n@echo no\r\n");
    let (code, out, err, _) = cmd_with(m, "cmd /c s a b");
    assert_eq!(out, "a\r\nb a\r\n");
    assert!(err.contains("nowhere"));
    assert_eq!(code, 1);
}

#[test]
fn interactive_and_k() {
    let mut m = mock();
    m.input.extend(["echo a".to_string(), "exit 2".into(), "echo b".into()]);
    let (code, out, _, _) = cmd_with(m, "cmd");
    assert_eq!(code, 2);
    assert_eq!(out, "\r\nC:\\work>a\r\n\r\nC:\\work>");
    let mut m = mock();
    m.input.push_back("echo then".into());
    let (code, out, _, _) = cmd_with(m, "cmd /q /k echo first");
    assert_eq!(code, 0);
    assert_eq!(out, "first\r\nthen\r\n");
}

#[test]
fn delimiters_after_builtin_names() {
    assert_eq!(builtin("echo.x"), Some(("echo", ".x")));
    assert_eq!(builtin("CD.."), Some(("cd", "..")));
    assert_eq!(builtin("echoes"), None);
    assert_eq!(builtin("set"), Some(("set", "")));
    assert_eq!(builtin("exit3"), None);
    assert_eq!(first_word("\"a b\" c"), ("\"a b\"", " c"));
}
