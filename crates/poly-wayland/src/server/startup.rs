//! Startup feedback (ADR 0167): the pointer is busy from the moment the
//! shell starts an app until the app has a window.
//!
//! [`crate::launch::Launches`] keeps the books; this is its place in the
//! server:
//!
//! - the shell [begins a launch](Server::begin_launch) and gets a token to
//!   put in the app's environment (`XDG_ACTIVATION_TOKEN`,
//!   `DESKTOP_STARTUP_ID`); a program that starts others itself (Files)
//!   [adopts](Server::adopt_launch) a token it made;
//! - it says which [process](Server::launch_pid) the app is;
//! - a client that activates a surface with the token
//!   (`xdg_activation_v1.activate`, `extras.rs`), or the first toplevel that
//!   a client of that process (or one it started) maps, ends the launch;
//!   X and Windows windows are reported by the compositor
//!   ([`Server::launch_window_mapped`]);
//! - [`Server::launch_clock`] tells the server the time, which ends the
//!   launches that waited too long, and turns the spinner;
//! - while a launch waits the pointer is the busy look ([`CursorLook::Busy`])
//!   everywhere a client has not set its own cursor (`cursor.rs`).

use super::{ClientId, Server};
use crate::compose::BUSY_PHASES;
use crate::launch::{Event, LaunchId};
use alloc::string::String;
use alloc::vec::Vec;

/// How long the spinner stays on one of its [`BUSY_PHASES`] pictures (ms):
/// a turn takes about a second.
pub const BUSY_STEP_MS: u64 = 60;

impl Server {
    /// How to find a process's parent, for windows of the children of an
    /// app that was launched.
    pub fn set_pid_parent(&mut self, f: fn(u32) -> Option<u32>) {
        self.launches.set_parent_lookup(f);
    }

    /// The process on the other end of client `c`'s socket.
    pub fn set_client_pid(&mut self, c: ClientId, pid: u32) {
        if pid != 0 {
            self.client_pids.insert(c, pid);
        }
    }

    /// The time, in milliseconds on any steady clock: launches that waited
    /// [`crate::launch::TIMEOUT_MS`] end, the spinner turns. Call it once
    /// each time round the loop, before the requests are read.
    pub fn launch_clock(&mut self, now_ms: u64) {
        self.launch_now = now_ms;
        self.launches.expire(now_ms);
        self.launches_changed();
    }

    /// The shell is starting the app `name`: the token to give it.
    pub fn begin_launch(&mut self, name: &str) -> (LaunchId, String) {
        let token = self.activation_token();
        let id = self.launches.begin(name, &token, self.launch_now);
        self.launches_changed();
        (id, token)
    }

    /// A program that starts apps itself is starting `name` with `token`
    /// (which it made up; clients may activate with it like with ours).
    /// Starting the same token twice is one launch.
    pub fn adopt_launch(&mut self, name: &str, token: &str) -> LaunchId {
        if let Some(id) = self.launches.by_token(token) {
            return id;
        }
        self.remember_token(token);
        let id = self.launches.begin(name, token, self.launch_now);
        self.launches_changed();
        id
    }

    /// The launch's app runs as process `pid`.
    pub fn launch_pid(&mut self, id: LaunchId, pid: u32) {
        self.launches.add_pid(id, pid);
    }

    /// [`Server::launch_pid`] for a launch known by its token.
    pub fn launch_pid_by_token(&mut self, token: &str, pid: u32) {
        if let Some(id) = self.launches.by_token(token) {
            self.launches.add_pid(id, pid);
        }
    }

    /// The app could not be started.
    pub fn cancel_launch(&mut self, id: LaunchId) {
        self.launches.cancel(id, self.launch_now);
        self.launches_changed();
    }

    /// [`Server::cancel_launch`] for a launch known by its token.
    pub fn cancel_launch_token(&mut self, token: &str) {
        if let Some(id) = self.launches.by_token(token) {
            self.cancel_launch(id);
        }
    }

    /// A launched process ended with `status`.
    pub fn launch_exited(&mut self, pid: u32, status: i32) {
        self.launches.process_exited(pid, status, self.launch_now);
        self.launches_changed();
    }

    /// A window was mapped by process `pid` (an X window by its
    /// `_NET_WM_PID`, a Windows window by its process).
    pub fn launch_window_mapped(&mut self, pid: u32) {
        if self.launches.window_mapped(pid, self.launch_now) > 0 {
            self.launches_changed();
        }
    }

    /// Client `c` mapped a toplevel.
    pub(super) fn client_window_mapped(&mut self, c: ClientId) {
        if let Some(&pid) = self.client_pids.get(&c) {
            self.launch_window_mapped(pid);
        }
    }

    /// A client activated a surface with `token`: true if that ended a
    /// launch.
    pub(super) fn token_activated(&mut self, token: &str) -> bool {
        let ended = self.launches.token_used(token, self.launch_now);
        if ended {
            self.launches_changed();
        }
        ended
    }

    /// Whether the pointer is busy: a launch waits for its window.
    pub fn launch_busy(&self) -> bool {
        self.launches.busy()
    }

    /// How many launches wait for a window.
    pub fn launches_pending(&self) -> usize {
        self.launches.pending()
    }

    /// Milliseconds until the server needs [`Server::launch_clock`] again
    /// (the spinner's next picture, or a timeout), if a launch waits.
    pub fn launch_wait_ms(&self) -> Option<u64> {
        if !self.launches.busy() {
            return None;
        }
        let turn = BUSY_STEP_MS - self.launch_now % BUSY_STEP_MS;
        let timeout = self.launches.next_deadline().map_or(turn, |d| d.saturating_sub(self.launch_now));
        Some(turn.min(timeout).max(1))
    }

    /// What happened to the launches since the last call.
    pub fn take_launch_events(&mut self) -> Vec<Event> {
        self.launches.take_events()
    }

    /// The spinner's picture now.
    pub(super) fn busy_phase(&self) -> u8 {
        ((self.launch_now / BUSY_STEP_MS) % u64::from(BUSY_PHASES)) as u8
    }

    /// Launches began or ended or time passed: the pointer follows if it is
    /// the desktop's (a client's own cursor stays).
    fn launches_changed(&mut self) {
        if self.seat.cursor.is_none() {
            self.update_cursor_look();
        }
    }
}
