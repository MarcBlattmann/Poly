//! Telling the desktop that this app is starting another one (ADR 0167):
//! the busy pointer shows from then until the new app has a window.
//!
//! The shell does this itself for what the launcher starts. A program that
//! starts apps on its own, like Files opening a document, writes to the
//! compositor's control pipe (`$POLY_COMPOSITOR_CONTROL`, set for every
//! client): `launch <token> <name>` before the start, `pid <token> <pid>`
//! after it, `cancel <token>` if it failed ([`Control`]). The token goes to
//! the new app as `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`, so a
//! toolkit that activates its window with it ends the wait at once, and the
//! pid ends it at the first window of the process or its children.
//!
//! [`Startup`] is the program's side without I/O in it: the writing goes
//! through a function, so the lines are tested on the host.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_wayland::launch::Control;

/// The variables a new app finds its token in.
pub const TOKEN_VARS: [&str; 2] = ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"];

/// Where lines go: the control pipe, or a test's list.
type Sink = Box<dyn FnMut(&str)>;

/// One app being started.
pub struct Startup {
    token: String,
    sink: Sink,
}

impl core::fmt::Debug for Startup {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Startup").field("token", &self.token).finish_non_exhaustive()
    }
}

/// The token for the `seq`th start of process `pid`: unique on the desktop
/// while the process lives.
pub fn token_for(pid: u32, seq: u32) -> String {
    format!("poly-launch-{pid}-{seq}")
}

/// What the desktop calls the app: the program's file name.
pub fn name_of(program: &str) -> &str {
    program.rsplit('/').find(|s| !s.is_empty()).unwrap_or(program)
}

impl Startup {
    /// About to start `name`: tell the desktop through `sink` (which gets
    /// whole lines, newline included).
    pub fn with_sink(name: &str, pid: u32, seq: u32, mut sink: Sink) -> Startup {
        let token = token_for(pid, seq);
        // One line: no control characters in the name.
        let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        sink(&Control::Launch { token: token.clone(), name }.line());
        Startup { token, sink }
    }

    /// About to start the program `program` from this process, telling the
    /// compositor through its control pipe (if there is one: without it the
    /// app is started all the same).
    pub fn begin(program: &str) -> Startup {
        static SEQ: AtomicU32 = AtomicU32::new(1);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let pipe = poly_rt::env::var("POLY_COMPOSITOR_CONTROL").filter(|p| p.starts_with('/'));
        let file = pipe.and_then(|p| {
            poly_rt::fs::File::open_with(p, poly_rt::abi::open::WRITE | poly_rt::abi::open::NONBLOCK, 0).ok()
        });
        let sink: Sink = Box::new(move |line| {
            if let Some(f) = &file {
                // A full or closed pipe loses the line: the wait ends by
                // itself.
                let _ = f.write_all(line.as_bytes());
            }
        });
        Startup::with_sink(name_of(program), poly_rt::process::id(), seq, sink)
    }

    /// The token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The environment variables for the new app, `NAME=value`.
    pub fn environment(&self) -> [String; 2] {
        TOKEN_VARS.map(|name| format!("{name}={}", self.token))
    }

    /// The app is running as process `pid`.
    pub fn started(&mut self, pid: u32) {
        (self.sink)(&Control::Pid { token: self.token.clone(), pid }.line());
    }

    /// The app could not be started.
    pub fn failed(&mut self) {
        (self.sink)(&Control::Cancel { token: self.token.clone() }.line());
    }
}

/// Whether `var` (`NAME=value`) is one of [`TOKEN_VARS`]: an app that was
/// itself launched must not pass its own token on.
pub fn is_token_var(var: &str) -> bool {
    TOKEN_VARS.iter().any(|name| var.strip_prefix(name).is_some_and(|rest| rest.starts_with('=')))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    fn recorded() -> (Sink, Rc<RefCell<Vec<String>>>) {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let l = Rc::clone(&lines);
        (Box::new(move |s| l.borrow_mut().push(s.into())), lines)
    }

    #[test]
    fn a_start_is_announced_then_its_process() {
        let (sink, lines) = recorded();
        let mut s = Startup::with_sink("viewer", 31, 4, sink);
        assert_eq!(s.token(), "poly-launch-31-4");
        s.started(77);
        assert_eq!(*lines.borrow(), ["launch poly-launch-31-4 viewer\n", "pid poly-launch-31-4 77\n"]);
        // What the compositor reads back.
        assert_eq!(
            Control::parse(&lines.borrow()[0]),
            Some(Control::Launch { token: "poly-launch-31-4".into(), name: "viewer".into() })
        );
    }

    #[test]
    fn a_failed_start_is_cancelled() {
        let (sink, lines) = recorded();
        let mut s = Startup::with_sink("nope", 5, 1, sink);
        s.failed();
        assert_eq!(lines.borrow()[1], "cancel poly-launch-5-1\n");
    }

    #[test]
    fn the_token_goes_to_the_app_in_both_variables() {
        let (sink, _) = recorded();
        let s = Startup::with_sink("a", 1, 2, sink);
        assert_eq!(s.environment(), ["XDG_ACTIVATION_TOKEN=poly-launch-1-2", "DESKTOP_STARTUP_ID=poly-launch-1-2"]);
    }

    #[test]
    fn an_old_token_is_not_passed_on() {
        assert!(is_token_var("XDG_ACTIVATION_TOKEN=poly-activation-0-3"));
        assert!(is_token_var("DESKTOP_STARTUP_ID=x"));
        assert!(!is_token_var("XDG_ACTIVATION_TOKEN_X=1"));
        assert!(!is_token_var("HOME=/root"));
    }

    #[test]
    fn the_name_is_the_programs_file_name() {
        assert_eq!(name_of("/bin/poly-terminal"), "poly-terminal");
        assert_eq!(name_of("viewer"), "viewer");
        assert_eq!(name_of("/usr/bin/"), "bin");
    }
}
