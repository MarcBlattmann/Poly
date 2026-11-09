//! Processes: [`exit`], [`id`], and [`Command`] to start programs of any
//! world (native, Linux, Windows), wait for them, learn when they end
//! ([`Child::process_handle`]) and end them.

use crate::abi::{self, ExitStatus, Handle, HandleMap, STDIO_NONE, SpawnRequest, Str};
use crate::fs::File;
use crate::{Error, Result, sys};
use alloc::string::String;
use alloc::vec::Vec;

/// End the process with status `code & 0xff`.
pub fn exit(code: i32) -> ! {
    sys::exit(code)
}

/// This process's id.
pub fn id() -> u32 {
    sys::process_id()
}

/// The user id native processes run as: 0. Native processes have no
/// credentials of their own yet (ADR 0125); what they create is root's.
pub fn uid() -> u32 {
    0
}

/// Whether a process with id `pid` exists.
pub fn alive(pid: u32) -> bool {
    pid != 0 && sys::signal(pid, abi::signal::PROBE).is_ok()
}

/// Which handle a child gets as one of its standard handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stdio {
    /// The caller's handle of the same number.
    Inherit,
    /// The caller's handle `h` (shared, as `dup` shares it).
    Handle(Handle),
    /// None.
    Null,
}

/// A program to start: its path, arguments (`args[0]` is the path unless
/// [`arg0`](Command::arg0) sets it), environment, standard handles and
/// extra handles.
#[derive(Clone, Debug)]
pub struct Command {
    path: String,
    args: Vec<String>,
    env: Vec<String>,
    stdio: [Stdio; 3],
    handles: Vec<HandleMap>,
}

impl Command {
    pub fn new(path: &str) -> Command {
        Command {
            path: path.into(),
            args: alloc::vec![path.into()],
            env: Vec::new(),
            stdio: [Stdio::Inherit; 3],
            handles: Vec::new(),
        }
    }

    /// Set `args[0]`, the name the program sees itself started by.
    pub fn arg0(&mut self, name: &str) -> &mut Command {
        self.args[0] = name.into();
        self
    }

    pub fn arg(&mut self, arg: &str) -> &mut Command {
        self.args.push(arg.into());
        self
    }

    pub fn args<'a>(&mut self, args: impl IntoIterator<Item = &'a str>) -> &mut Command {
        self.args.extend(args.into_iter().map(String::from));
        self
    }

    /// Add `name=value` to the environment (which starts empty).
    pub fn env(&mut self, name: &str, value: &str) -> &mut Command {
        self.env.push(alloc::format!("{name}={value}"));
        self
    }

    /// Add `NAME=value` strings to the environment.
    pub fn envs<'a>(&mut self, vars: impl IntoIterator<Item = &'a str>) -> &mut Command {
        self.env.extend(vars.into_iter().map(String::from));
        self
    }

    pub fn stdin(&mut self, s: Stdio) -> &mut Command {
        self.stdio[0] = s;
        self
    }

    pub fn stdout(&mut self, s: Stdio) -> &mut Command {
        self.stdio[1] = s;
        self
    }

    pub fn stderr(&mut self, s: Stdio) -> &mut Command {
        self.stdio[2] = s;
        self
    }

    /// Give the child the caller's handle `h` under the same number (3 or
    /// more): a descriptor a program is told about in its arguments or
    /// environment.
    pub fn keep(&mut self, h: Handle) -> &mut Command {
        self.handles.push(HandleMap { from: h, to: h });
        self
    }

    /// The `spawn` request for this command, pointing into `self` and the
    /// two [`Str`] arrays (which must outlive the request's use).
    fn request(&self, args: &mut Vec<Str>, env: &mut Vec<Str>) -> SpawnRequest {
        let s = |v: &String| Str { ptr: v.as_ptr() as u64, len: v.len() as u64 };
        *args = self.args.iter().map(s).collect();
        *env = self.env.iter().map(s).collect();
        let mut stdio = [0; 3];
        for (i, (slot, how)) in stdio.iter_mut().zip(self.stdio).enumerate() {
            *slot = match how {
                Stdio::Inherit => i as u32,
                Stdio::Handle(h) => h,
                Stdio::Null => STDIO_NONE,
            };
        }
        SpawnRequest {
            path: s(&self.path),
            args: args.as_ptr() as u64,
            arg_count: args.len() as u64,
            env: env.as_ptr() as u64,
            env_count: env.len() as u64,
            stdio,
            flags: 0,
            handles: self.handles.as_ptr() as u64,
            handle_count: self.handles.len() as u64,
        }
    }

    /// Start the program.
    pub fn spawn(&self) -> Result<Child> {
        let (mut args, mut env) = (Vec::new(), Vec::new());
        let request = self.request(&mut args, &mut env);
        // SAFETY: the request points into `self`, `args` and `env`, all
        // alive and unchanged for the call.
        let pid = unsafe { sys::spawn(&request) }?;
        Ok(Child { pid })
    }

    /// Start the program and wait for it.
    pub fn status(&self) -> Result<ExitStatus> {
        self.spawn()?.wait()
    }
}

/// A started child process.
#[derive(Debug)]
pub struct Child {
    pid: u32,
}

impl Child {
    /// The child with id `pid` (one this process spawned).
    pub fn from_pid(pid: u32) -> Child {
        Child { pid }
    }

    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Wait for it to end.
    pub fn wait(self) -> Result<ExitStatus> {
        sys::wait(self.pid, false)
    }

    /// How it ended, if it has (it is reaped then); `None` while it runs.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>> {
        match sys::wait(self.pid, true) {
            Ok(s) => Ok(Some(s)),
            Err(Error::WouldBlock) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A handle that turns readable when it ends (to `poll` with others).
    pub fn process_handle(&self) -> Result<File> {
        sys::process_handle(self.pid).map(File::from_handle)
    }

    /// Ask it to end (a Linux child gets `SIGTERM`).
    pub fn terminate(&self) -> Result<()> {
        sys::signal(self.pid, abi::signal::TERMINATE)
    }

    /// End it now (a Linux child gets `SIGKILL`).
    pub fn kill(&self) -> Result<()> {
        sys::signal(self.pid, abi::signal::KILL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_points_at_the_command() {
        let mut c = Command::new("/bin/hello");
        c.arg("--twice").env("HOME", "/").stdout(Stdio::Handle(5)).stderr(Stdio::Null).keep(7).keep(9);
        let (mut args, mut env) = (Vec::new(), Vec::new());
        let r = c.request(&mut args, &mut env);
        assert_eq!(r.path.len, 10);
        assert_eq!((r.arg_count, r.env_count, r.handle_count), (2, 1, 2));
        assert_eq!(r.stdio, [0, 5, STDIO_NONE]);
        assert_eq!(args[1].len, 7);
        assert_eq!(env[0].len, "HOME=/".len() as u64);
        assert_eq!(r.args, args.as_ptr() as u64);
        assert_eq!(c.handles[1], HandleMap { from: 9, to: 9 });
        assert_eq!(c.status(), Err(Error::Unsupported), "no kernel on the host");
        assert!(!alive(1));
    }
}
