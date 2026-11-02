//! Processes: [`exit`], [`id`], and [`Command`] to start programs of any
//! world (native, Linux, Windows) and wait for them.

use crate::abi::{ExitStatus, Handle, STDIO_NONE, SpawnRequest, Str};
use crate::{Result, sys};
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
/// [`arg0`](Command::arg0) sets it), environment and standard handles.
#[derive(Clone, Debug)]
pub struct Command {
    path: String,
    args: Vec<String>,
    env: Vec<String>,
    stdio: [Stdio; 3],
}

impl Command {
    pub fn new(path: &str) -> Command {
        Command { path: path.into(), args: alloc::vec![path.into()], env: Vec::new(), stdio: [Stdio::Inherit; 3] }
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
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Wait for it to end.
    pub fn wait(self) -> Result<ExitStatus> {
        sys::wait(self.pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn a_request_points_at_the_command() {
        let mut c = Command::new("/bin/hello");
        c.arg("--twice").env("HOME", "/").stdout(Stdio::Handle(5)).stderr(Stdio::Null);
        let (mut args, mut env) = (Vec::new(), Vec::new());
        let r = c.request(&mut args, &mut env);
        assert_eq!(r.path.len, 10);
        assert_eq!((r.arg_count, r.env_count), (2, 1));
        assert_eq!(r.stdio, [0, 5, STDIO_NONE]);
        assert_eq!(args[1].len, 7);
        assert_eq!(env[0].len, "HOME=/".len() as u64);
        assert_eq!(r.args, args.as_ptr() as u64);
        assert_eq!(c.status(), Err(Error::Unsupported), "no kernel on the host");
    }
}
