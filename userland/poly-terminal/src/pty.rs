//! The pseudo-terminal and the shell on it, through the Linux
//! personality's devices (`/dev/ptmx`, `/dev/pts/<n>`; pts(4)): the master
//! opened, unlocked (`TIOCSPTLCK`) and numbered (`TIOCGPTN`), the slave
//! opened and given to the shell as its standard handles, the window's
//! size set with `TIOCSWINSZ` (which sends `SIGWINCH` to the foreground
//! group).
//!
//! The shell gets its **controlling terminal** without anything new in the
//! kernel: a Linux program a native process starts is the root of its own
//! process tree, so it leads a session of its own (ADR 0126), and a session
//! leader without a controlling terminal that opens a terminal without
//! `O_NOCTTY` makes it its controlling terminal (POSIX; `poly-tty`). The
//! handles it is given were opened by the terminal, so the shell first
//! opens its terminal itself: it runs `sh -c 'exec "$@" <>/dev/pts/N >&0
//! 2>&0' sh <shell> -i`, and the `exec` keeps the process, so the
//! interactive shell is the session leader with its terminal, and job
//! control and `^C` work.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use poly_rt::abi::{self, open};
use poly_rt::fs::File;
use poly_rt::process::{Child, Command, Stdio};
use poly_rt::sys;

/// The ioctl requests (Linux's `asm-generic/ioctls.h`, which the Linux
/// personality's terminals take).
pub const TIOCSWINSZ: u32 = 0x5414;
pub const TIOCGPTN: u32 = 0x8004_5430;
pub const TIOCSPTLCK: u32 = 0x4004_5431;

/// The shells tried in order when `$SHELL` names none that is there.
pub const SHELLS: [&str; 3] = ["/bin/bash", "/usr/bin/bash", "/bin/sh"];

/// The shell to run: `$SHELL` if it exists, else the first of [`SHELLS`]
/// that does.
pub fn choose_shell(env_shell: Option<&str>, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    env_shell
        .filter(|s| s.starts_with('/') && exists(s))
        .map(String::from)
        .or_else(|| SHELLS.iter().find(|s| exists(s)).map(|s| s.to_string()))
}

/// The program and arguments that start `program args` on terminal `pts`
/// through `sh` (see the module documentation), in directory `dir`.
pub fn shell_command(sh: &str, pts: &str, dir: Option<&str>, program: &[String]) -> Vec<String> {
    let cd = match dir {
        Some(_) => "cd \"$POLY_TERMINAL_DIR\" 2>/dev/null || cd; ",
        None => "cd 2>/dev/null; ",
    };
    let script = format!("{cd}exec \"$@\" <>{pts} >&0 2>&0");
    let mut argv: Vec<String> = alloc::vec![sh.into(), "-c".into(), script, "sh".into()];
    argv.extend(program.iter().cloned());
    argv
}

/// The environment the shell gets: the terminal's own, with `TERM`,
/// `COLORTERM`, a `PATH`, `HOME` and `SHELL` set, and what the window
/// system set for the terminal itself taken out of what should not leak.
pub fn shell_env(ours: &[(String, String)], shell: &str, dir: Option<&str>) -> Vec<String> {
    let skip = ["TERM", "COLORTERM", "SHELL", "POLY_TERMINAL_DIR", "COLUMNS", "LINES"];
    let mut env: Vec<String> =
        ours.iter().filter(|(k, _)| !skip.contains(&k.as_str())).map(|(k, v)| format!("{k}={v}")).collect();
    let has = |k: &str| ours.iter().any(|(n, _)| n == k);
    env.push("TERM=xterm-256color".into());
    env.push("COLORTERM=truecolor".into());
    env.push(format!("SHELL={shell}"));
    env.push("TERM_PROGRAM=poly-terminal".into());
    if !has("PATH") {
        env.push("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());
    }
    if !has("HOME") {
        env.push("HOME=/root".into());
    }
    if let Some(d) = dir {
        env.push(format!("POLY_TERMINAL_DIR={d}"));
    }
    env
}

/// `struct winsize` (`asm-generic/termios.h`): rows, columns, pixels.
pub fn winsize(cols: u16, rows: u16, width_px: u16, height_px: u16) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0..2].copy_from_slice(&rows.to_le_bytes());
    b[2..4].copy_from_slice(&cols.to_le_bytes());
    b[4..6].copy_from_slice(&width_px.to_le_bytes());
    b[6..8].copy_from_slice(&height_px.to_le_bytes());
    b
}

/// What to run, and how, once the window's size is known (see
/// [`Launch::start`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// The `sh` that opens the terminal for the program.
    pub sh: String,
    /// The user's shell (for `SHELL`).
    pub shell: String,
    /// The program and its arguments.
    pub program: Vec<String>,
    pub dir: Option<String>,
    /// The terminal's own environment.
    pub ours: Vec<(String, String)>,
}

impl Launch {
    /// Open a pseudo-terminal of the given size and start the program on it.
    ///
    /// The size must be the window's real one: a shell that already printed
    /// its prompt when the terminal is resized (`SIGWINCH`) redraws it, and
    /// the prompt shows twice. This is why the terminal starts the shell
    /// only at its first layout, not when the process starts.
    pub fn start(&self, cols: u16, rows: u16, width_px: u16, height_px: u16) -> Result<Pty, String> {
        let (mut p, slave) =
            Pty::open(cols, rows, width_px, height_px).map_err(|e| format!("No pseudo-terminal: {e}"))?;
        let argv = shell_command(&self.sh, &p.pts, self.dir.as_deref(), &self.program);
        let env = shell_env(&self.ours, &self.shell, self.dir.as_deref());
        p.spawn(&argv, &env, &slave).map_err(|e| format!("Could not start {}: {e}", self.shell))?;
        poly_rt::println!("poly-terminal: {} on {} ({cols}x{rows})", self.program.join(" "), p.pts);
        Ok(p)
    }
}

/// A pseudo-terminal's master and the child on its slave.
#[derive(Debug)]
pub struct Pty {
    pub master: File,
    pub child: Option<Child>,
    pub pts: String,
}

impl Pty {
    /// Open a pseudo-terminal of `cols` × `rows` (`width_px` × `height_px`
    /// pixels, 0 when not known).
    pub fn open(cols: u16, rows: u16, width_px: u16, height_px: u16) -> Result<(Pty, File), String> {
        let master = File::open_with("/dev/ptmx", open::READ | open::WRITE | open::NONBLOCK, 0)
            .map_err(|e| format!("/dev/ptmx: {e}"))?;
        let unlock = 0u32;
        // SAFETY: `TIOCSPTLCK` reads an int from the address, which points
        // to a live local for the call.
        unsafe { sys::control(master.handle(), TIOCSPTLCK, &unlock as *const u32 as u64) }
            .map_err(|e| format!("unlockpt: {e}"))?;
        let mut n = 0u32;
        // SAFETY: `TIOCGPTN` writes an unsigned int to the address, a live
        // local for the call.
        unsafe { sys::control(master.handle(), TIOCGPTN, &mut n as *mut u32 as u64) }
            .map_err(|e| format!("ptsname: {e}"))?;
        let pts = format!("/dev/pts/{n}");
        let slave = File::open_with(&pts, open::READ | open::WRITE, 0).map_err(|e| format!("{pts}: {e}"))?;
        let pty = Pty { master, child: None, pts };
        pty.resize(cols, rows, width_px, height_px);
        Ok((pty, slave))
    }

    /// Tell the terminal (and so the shell, by `SIGWINCH`) its size.
    pub fn resize(&self, cols: u16, rows: u16, width_px: u16, height_px: u16) {
        let ws = winsize(cols, rows, width_px, height_px);
        // SAFETY: `TIOCSWINSZ` reads a `struct winsize` (8 bytes) from the
        // address, a live local for the call.
        let _ = unsafe { sys::control(self.master.handle(), TIOCSWINSZ, ws.as_ptr() as u64) };
    }

    /// Start `argv` (from [`shell_command`]) with `env` on the slave.
    pub fn spawn(&mut self, argv: &[String], env: &[String], slave: &File) -> Result<(), String> {
        let program = argv.first().ok_or("nothing to run")?;
        let mut cmd = Command::new(program);
        cmd.args(argv.iter().skip(1).map(String::as_str)).envs(env.iter().map(String::as_str));
        let h = slave.handle();
        cmd.stdin(Stdio::Handle(h)).stdout(Stdio::Handle(h)).stderr(Stdio::Handle(h));
        self.child = Some(cmd.spawn().map_err(|e| format!("{program}: {e}"))?);
        Ok(())
    }

    /// Read what the shell wrote (empty: nothing now; `None`: it is gone).
    pub fn read(&self, buf: &mut [u8]) -> Option<usize> {
        match sys::read(self.master.handle(), buf) {
            Ok(0) => None,
            Ok(n) => Some(n),
            Err(abi::Error::WouldBlock | abi::Error::Interrupted) => Some(0),
            Err(_) => None,
        }
    }

    /// Write `bytes` for the shell; how many went (the rest waits). An
    /// error: the shell is gone.
    pub fn write(&self, bytes: &[u8]) -> Result<usize, abi::Error> {
        match sys::write(self.master.handle(), bytes) {
            Ok(n) => Ok(n),
            Err(abi::Error::WouldBlock | abi::Error::Interrupted) => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Whether the child ended (reaping it).
    pub fn child_ended(&self) -> bool {
        self.child.as_ref().is_none_or(|c| c.try_wait().ok().flatten().is_some())
    }

    /// End the child (the window closed): `SIGHUP` reaches it when the
    /// master closes; a terminate makes sure.
    pub fn hang_up(&self) {
        if let Some(c) = &self.child {
            let _ = c.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn the_shell_is_shells_or_the_first_there() {
        let there = |p: &str| matches!(p, "/bin/sh" | "/usr/bin/zsh");
        assert_eq!(choose_shell(Some("/usr/bin/zsh"), &there).as_deref(), Some("/usr/bin/zsh"));
        assert_eq!(choose_shell(Some("/bin/fish"), &there).as_deref(), Some("/bin/sh"));
        assert_eq!(choose_shell(Some("zsh"), &there).as_deref(), Some("/bin/sh"), "not a path");
        assert_eq!(choose_shell(None, &|p| p == "/bin/bash").as_deref(), Some("/bin/bash"));
        assert_eq!(choose_shell(None, &|_| false), None);
    }

    #[test]
    fn the_shell_opens_its_terminal_itself() {
        let argv = shell_command("/bin/sh", "/dev/pts/3", None, &["/bin/bash".into(), "-i".into()]);
        assert_eq!(argv[..2], ["/bin/sh", "-c"]);
        assert_eq!(argv[2], "cd 2>/dev/null; exec \"$@\" <>/dev/pts/3 >&0 2>&0");
        assert_eq!(argv[3..], ["sh", "/bin/bash", "-i"]);
        let argv = shell_command("/bin/sh", "/dev/pts/0", Some("/tmp"), &["vim".into()]);
        assert!(argv[2].starts_with("cd \"$POLY_TERMINAL_DIR\""), "{}", argv[2]);
    }

    #[test]
    fn the_environment_names_the_terminal() {
        let ours = vec![("TERM".into(), "linux".into()), ("HOME".into(), "/home/m".into())];
        let env = shell_env(&ours, "/bin/sh", Some("/tmp/x y"));
        assert!(env.contains(&"TERM=xterm-256color".into()));
        assert!(!env.contains(&"TERM=linux".into()));
        assert!(env.contains(&"HOME=/home/m".into()));
        assert!(env.iter().any(|v| v.starts_with("PATH=")));
        assert!(env.contains(&"POLY_TERMINAL_DIR=/tmp/x y".into()));
        assert!(env.contains(&"SHELL=/bin/sh".into()));
    }

    #[test]
    fn winsize_is_rows_then_columns() {
        assert_eq!(winsize(80, 24, 640, 384), [24, 0, 80, 0, 0x80, 2, 0x80, 1]);
    }
}
