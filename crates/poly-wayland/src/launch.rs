//! Startup feedback: which launched apps have not shown a window yet
//! (ADR 0167).
//!
//! The shell starts an app and the desktop shows a busy pointer until the
//! app has a window, so a slow start is not mistaken for a click that did
//! nothing. [`Launches`] is the bookkeeping, with no clock of its own (the
//! caller passes the time in milliseconds), so everything here is tested on
//! the host. A launch is:
//!
//! - **begun** with a token, which the launcher hands to the app as
//!   `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`
//!   (xdg-activation-v1.xml, startup-notification), and then the process it
//!   started ([`Launches::add_pid`]);
//! - **finished** by the first of: a client activating a surface with the
//!   token ([`Launches::token_used`]); a window mapped by the process, or by
//!   a process it started, however far down ([`Launches::window_mapped`]:
//!   Wayland clients are known by their peer's pid, X windows by
//!   `_NET_WM_PID`, Windows windows by the process win32k reports); the
//!   process ending with a failure status ([`Launches::process_exited`]; a
//!   success may be a launcher that handed over to a running instance, so
//!   the wait goes on for [`EXIT_GRACE_MS`], which a program with no window
//!   to show, like a notification sender, would otherwise spin through
//!   [`TIMEOUT_MS`] for); the app being cancelled; or [`TIMEOUT_MS`] passing
//!   ([`Launches::expire`]).
//!
//! The pointer is busy while any launch is [`pending`](Launches::busy), so
//! several launches at once keep it busy until the last is done.
//! [`Launches::take_events`] says what happened, in words the compositor
//! logs ([`Event::describe`]).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// How long a launch may wait for a window before the busy pointer stops.
pub const TIMEOUT_MS: u64 = 30_000;
/// How long a launch still waits after its process ended well without a
/// window: a launcher that handed over to a running instance, whose window
/// shows shortly.
pub const EXIT_GRACE_MS: u64 = 3_000;
/// How many generations of parents are followed from a window's process.
const DEPTH: usize = 32;

/// A launch.
pub type LaunchId = u32;

/// Why a launch is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// A client activated a surface with the launch's token.
    Activated,
    /// A window was mapped by `pid`, the process or one it started.
    Window { pid: u32 },
    /// The process ended with a failure `status`, before any window.
    Exited { status: i32 },
    /// No window in [`TIMEOUT_MS`].
    TimedOut,
    /// The launcher could not start the app.
    Cancelled,
}

/// Something that happened to the launches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The app `name` is being started; `pending` launches are waiting.
    Started { id: LaunchId, name: String, pending: usize },
    /// The launch ended `after_ms` after it began; `pending` are still waiting.
    Finished { id: LaunchId, name: String, how: Ended, after_ms: u64, pending: usize },
}

impl Event {
    /// The compositor's log lines for it (a finish that leaves nothing
    /// waiting adds the one saying the busy pointer is off).
    pub fn describe(&self) -> Vec<String> {
        match self {
            Event::Started { name, pending, .. } => {
                alloc::vec![format!("busy pointer on: launching {name} ({pending} pending)")]
            }
            Event::Finished { name, how, after_ms, pending, .. } => {
                let why = match how {
                    Ended::Activated => format!("activated by its token after {after_ms} ms"),
                    Ended::Window { pid } => format!("window mapped by pid {pid} after {after_ms} ms"),
                    Ended::Exited { status } => format!("exited with status {status} after {after_ms} ms, no window"),
                    Ended::TimedOut => format!("timed out after {after_ms} ms, no window"),
                    Ended::Cancelled => format!("not started ({after_ms} ms)"),
                };
                let mut lines = alloc::vec![format!("launch {name}: {why} ({pending} pending)")];
                if *pending == 0 {
                    lines.push(String::from("busy pointer off"));
                }
                lines
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Launch {
    id: LaunchId,
    name: String,
    token: String,
    /// The processes the launcher started for it.
    pids: Vec<u32>,
    started: u64,
    /// When its process ended with status 0: the wait ends
    /// [`EXIT_GRACE_MS`] later.
    quiet_since: Option<u64>,
}

/// The launches waiting for a window.
#[derive(Debug)]
pub struct Launches {
    list: Vec<Launch>,
    next: LaunchId,
    parent_of: fn(u32) -> Option<u32>,
    events: Vec<Event>,
    timeout_ms: u64,
}

impl Default for Launches {
    fn default() -> Launches {
        Launches::new()
    }
}

impl Launches {
    pub fn new() -> Launches {
        Launches { list: Vec::new(), next: 1, parent_of: |_| None, events: Vec::new(), timeout_ms: TIMEOUT_MS }
    }

    /// How to find a process's parent (without it only the launched
    /// process itself is recognised).
    pub fn set_parent_lookup(&mut self, f: fn(u32) -> Option<u32>) {
        self.parent_of = f;
    }

    /// A shorter or longer wait than [`TIMEOUT_MS`].
    pub fn set_timeout(&mut self, ms: u64) {
        self.timeout_ms = ms;
    }

    /// The app `name` is being started with `token` at time `now` (ms).
    pub fn begin(&mut self, name: &str, token: &str, now: u64) -> LaunchId {
        let id = self.next;
        self.next += 1;
        self.list.push(Launch {
            id,
            name: name.into(),
            token: token.into(),
            pids: Vec::new(),
            started: now,
            quiet_since: None,
        });
        let pending = self.list.len();
        self.events.push(Event::Started { id, name: name.into(), pending });
        id
    }

    /// The launch's process, once it is started.
    pub fn add_pid(&mut self, id: LaunchId, pid: u32) {
        if let Some(l) = self.list.iter_mut().find(|l| l.id == id)
            && pid != 0
            && !l.pids.contains(&pid)
        {
            l.pids.push(pid);
        }
    }

    /// The launch with `token`, if it waits.
    pub fn by_token(&self, token: &str) -> Option<LaunchId> {
        self.list.iter().find(|l| l.token == token).map(|l| l.id)
    }

    /// A client activated a surface with `token`; true if a launch waited
    /// for it (it is over).
    pub fn token_used(&mut self, token: &str, now: u64) -> bool {
        match self.by_token(token) {
            Some(id) => self.finish(id, Ended::Activated, now),
            None => false,
        }
    }

    /// A window was mapped by process `pid`: every launch whose process is
    /// `pid` or one of its ancestors is over. Returns how many.
    pub fn window_mapped(&mut self, pid: u32, now: u64) -> usize {
        if pid == 0 || self.list.is_empty() {
            return 0;
        }
        let mut chain = alloc::vec![pid];
        while chain.len() < DEPTH {
            match (self.parent_of)(*chain.last().unwrap_or(&pid)) {
                Some(p) if p > 1 && !chain.contains(&p) => chain.push(p),
                _ => break,
            }
        }
        let done: Vec<LaunchId> =
            self.list.iter().filter(|l| l.pids.iter().any(|p| chain.contains(p))).map(|l| l.id).collect();
        for &id in &done {
            self.finish(id, Ended::Window { pid }, now);
        }
        done.len()
    }

    /// The launched process `pid` ended with `status`. A failure ends the
    /// launch (nothing will map a window); a success may have handed over
    /// to something else, so the wait goes on for [`EXIT_GRACE_MS`].
    pub fn process_exited(&mut self, pid: u32, status: i32, now: u64) {
        if status == 0 {
            for l in self.list.iter_mut().filter(|l| l.pids.contains(&pid)) {
                l.quiet_since.get_or_insert(now);
            }
            return;
        }
        let ids: Vec<LaunchId> = self.list.iter().filter(|l| l.pids.contains(&pid)).map(|l| l.id).collect();
        for id in ids {
            self.finish(id, Ended::Exited { status }, now);
        }
    }

    /// The app could not be started.
    pub fn cancel(&mut self, id: LaunchId, now: u64) {
        self.finish(id, Ended::Cancelled, now);
    }

    /// End the launches that waited [`TIMEOUT_MS`] (or the timeout set), or
    /// [`EXIT_GRACE_MS`] after their process ended well.
    pub fn expire(&mut self, now: u64) {
        let timeout = self.timeout_ms;
        let late: Vec<(LaunchId, Ended)> = self
            .list
            .iter()
            .filter_map(|l| {
                if now.saturating_sub(l.started) >= timeout {
                    Some((l.id, Ended::TimedOut))
                } else if l.quiet_since.is_some_and(|q| now.saturating_sub(q) >= EXIT_GRACE_MS) {
                    Some((l.id, Ended::Exited { status: 0 }))
                } else {
                    None
                }
            })
            .collect();
        for (id, how) in late {
            self.finish(id, how, now);
        }
    }

    fn finish(&mut self, id: LaunchId, how: Ended, now: u64) -> bool {
        let Some(i) = self.list.iter().position(|l| l.id == id) else { return false };
        let l = self.list.remove(i);
        let pending = self.list.len();
        self.events.push(Event::Finished { id, name: l.name, how, after_ms: now.saturating_sub(l.started), pending });
        true
    }

    /// Whether any launch waits for its window.
    pub fn busy(&self) -> bool {
        !self.list.is_empty()
    }

    /// How many launches wait.
    pub fn pending(&self) -> usize {
        self.list.len()
    }

    /// When the next launch ends for want of a window (ms), if any waits.
    pub fn next_deadline(&self) -> Option<u64> {
        let timeout = self.timeout_ms;
        self.list
            .iter()
            .map(|l| {
                let by_exit = l.quiet_since.map_or(u64::MAX, |q| q + EXIT_GRACE_MS);
                (l.started + timeout).min(by_exit)
            })
            .min()
    }

    /// What happened since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }
}

/// The parent's pid from the text of `/proc/<pid>/stat`: after the
/// command's closing parenthesis (it may hold spaces and parentheses) come
/// the state and the parent.
pub fn parent_from_stat(text: &str) -> Option<u32> {
    let rest = &text[text.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// A line the launching program writes to the compositor's control pipe
/// (`$POLY_COMPOSITOR_CONTROL`) to have its launch shown (an app that
/// starts others, like Files).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// `launch <token> <name>`: about to start the app `name`, which will
    /// find `token` in its environment.
    Launch { token: String, name: String },
    /// `pid <token> <pid>`: it is started.
    Pid { token: String, pid: u32 },
    /// `cancel <token>`: starting it failed.
    Cancel { token: String },
    /// `open <id>`: start the app the desktop's catalog has under this ID (a
    /// desktop file's name, `poly:<name>` for one of Poly's own), exactly
    /// as the launcher starts it (ADR 0175: Files' Applications folder).
    Open { id: String },
}

impl Control {
    /// Read a control line; `None` for any other (`dump`, noise).
    pub fn parse(line: &str) -> Option<Control> {
        let line = line.trim();
        let (word, rest) = line.split_once(' ')?;
        let rest = rest.trim();
        match word {
            "launch" => {
                let (token, name) = rest.split_once(' ')?;
                let (token, name) = (token.trim(), name.trim());
                (valid_token(token) && !name.is_empty())
                    .then(|| Control::Launch { token: token.into(), name: name.into() })
            }
            "pid" => {
                let (token, pid) = rest.split_once(' ')?;
                let pid = pid.trim().parse().ok()?;
                valid_token(token).then(|| Control::Pid { token: token.into(), pid })
            }
            "cancel" => valid_token(rest).then(|| Control::Cancel { token: rest.into() }),
            "open" => valid_app_id(rest).then(|| Control::Open { id: rest.into() }),
            _ => None,
        }
    }

    /// The line for it, as the launching program writes it.
    pub fn line(&self) -> String {
        match self {
            Control::Launch { token, name } => format!("launch {token} {name}\n"),
            Control::Pid { token, pid } => format!("pid {token} {pid}\n"),
            Control::Cancel { token } => format!("cancel {token}\n"),
            Control::Open { id } => format!("open {id}\n"),
        }
    }
}

/// Tokens are words of printable characters (they travel in environment
/// variables and on one line).
fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= 96 && t.bytes().all(|b| b.is_ascii_graphic())
}

/// An app's ID is a file name or `poly:<name>`: one line, no control
/// characters, and nothing around it that reading the line would trim.
fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 255 && id.trim() == id && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process tree: 100 -> 101 -> 102, and 200 alone.
    fn tree(p: u32) -> Option<u32> {
        match p {
            102 => Some(101),
            101 => Some(100),
            100 => Some(1),
            _ => None,
        }
    }

    fn launches() -> Launches {
        let mut l = Launches::new();
        l.set_parent_lookup(tree);
        l
    }

    #[test]
    fn a_launch_waits_until_its_token_is_used() {
        let mut l = launches();
        assert!(!l.busy());
        let id = l.begin("Files", "tok-1", 1_000);
        l.add_pid(id, 100);
        assert!(l.busy());
        assert!(!l.token_used("other", 1_200));
        assert!(l.busy());
        assert!(l.token_used("tok-1", 1_500));
        assert!(!l.busy());
        // Used once.
        assert!(!l.token_used("tok-1", 1_600));
        let events = l.take_events();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            Event::Finished { id, name: "Files".into(), how: Ended::Activated, after_ms: 500, pending: 0 }
        );
    }

    #[test]
    fn a_window_of_the_process_or_of_its_descendants_ends_it() {
        let mut l = launches();
        let id = l.begin("Browser", "t", 0);
        l.add_pid(id, 100);
        // Not related.
        assert_eq!(l.window_mapped(200, 10), 0);
        assert_eq!(l.window_mapped(0, 10), 0);
        assert!(l.busy());
        // A grandchild.
        assert_eq!(l.window_mapped(102, 840), 1);
        assert!(!l.busy());
        let Some(Event::Finished { how, after_ms, .. }) = l.take_events().pop() else { panic!("finished") };
        assert_eq!((how, after_ms), (Ended::Window { pid: 102 }, 840));
        // The launched process itself.
        let id = l.begin("Terminal", "t2", 0);
        l.add_pid(id, 200);
        assert_eq!(l.window_mapped(200, 5), 1);
    }

    #[test]
    fn without_a_parent_lookup_only_the_process_matches() {
        let mut l = Launches::new();
        let id = l.begin("A", "t", 0);
        l.add_pid(id, 100);
        assert_eq!(l.window_mapped(102, 1), 0);
        assert_eq!(l.window_mapped(100, 2), 1);
    }

    #[test]
    fn the_ancestor_walk_stops_on_a_loop() {
        fn looped(p: u32) -> Option<u32> {
            Some(if p == 5 { 6 } else { 5 })
        }
        let mut l = Launches::new();
        l.set_parent_lookup(looped);
        let id = l.begin("A", "t", 0);
        l.add_pid(id, 77);
        assert_eq!(l.window_mapped(5, 1), 0);
    }

    #[test]
    fn several_launches_keep_it_busy_until_the_last() {
        let mut l = launches();
        let a = l.begin("A", "ta", 0);
        let b = l.begin("B", "tb", 100);
        let c = l.begin("C", "tc", 200);
        l.add_pid(a, 100);
        l.add_pid(b, 200);
        l.add_pid(c, 300);
        assert_eq!(l.pending(), 3);
        assert_eq!(l.window_mapped(101, 300), 1);
        assert_eq!(l.pending(), 2);
        assert!(l.token_used("tc", 400));
        assert!(l.busy());
        assert_eq!(l.window_mapped(200, 500), 1);
        assert!(!l.busy());
        let events = l.take_events();
        let left: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                Event::Finished { pending, .. } => Some(*pending),
                _ => None,
            })
            .collect();
        assert_eq!(left, [2, 1, 0]);
        let started: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                Event::Started { pending, .. } => Some(*pending),
                _ => None,
            })
            .collect();
        assert_eq!(started, [1, 2, 3]);
    }

    #[test]
    fn a_launch_times_out() {
        let mut l = launches();
        l.begin("Slow", "t1", 1_000);
        l.begin("Later", "t2", 20_000);
        assert_eq!(l.next_deadline(), Some(1_000 + TIMEOUT_MS));
        l.expire(30_999);
        assert_eq!(l.pending(), 2);
        l.expire(31_000);
        assert_eq!(l.pending(), 1);
        assert_eq!(l.next_deadline(), Some(20_000 + TIMEOUT_MS));
        let events = l.take_events();
        let Some(Event::Finished { how, after_ms, name, pending, .. }) = events.last() else { panic!("finished") };
        assert_eq!((how, *after_ms, name.as_str(), *pending), (&Ended::TimedOut, 30_000, "Slow", 1));
        l.expire(50_000);
        assert!(!l.busy());
        assert_eq!(l.next_deadline(), None);
    }

    #[test]
    fn a_shorter_timeout_can_be_set() {
        let mut l = launches();
        l.set_timeout(2_000);
        l.begin("A", "t", 0);
        l.expire(1_999);
        assert!(l.busy());
        l.expire(2_000);
        assert!(!l.busy());
    }

    #[test]
    fn a_failed_process_ends_the_wait_a_clean_exit_does_not() {
        let mut l = launches();
        let a = l.begin("Opens in a running app", "ta", 0);
        let b = l.begin("Crashes", "tb", 0);
        l.add_pid(a, 100);
        l.add_pid(b, 200);
        l.process_exited(100, 0, 50);
        assert_eq!(l.pending(), 2);
        l.process_exited(200, 139, 60);
        assert_eq!(l.pending(), 1);
        // The running instance's window is a child of nobody we know: the
        // token ends it, or the grace period after the clean exit.
        assert_eq!(l.next_deadline(), Some(50 + EXIT_GRACE_MS));
        l.expire(50 + EXIT_GRACE_MS - 1);
        assert!(l.busy());
        l.expire(50 + EXIT_GRACE_MS);
        assert!(!l.busy());
        let Some(Event::Finished { how, .. }) = l.take_events().pop() else { panic!("finished") };
        assert_eq!(how, Ended::Exited { status: 0 });
    }

    #[test]
    fn a_window_in_the_grace_period_still_ends_it_as_one() {
        let mut l = launches();
        let id = l.begin("Handing over", "t", 0);
        l.add_pid(id, 100);
        l.process_exited(100, 0, 20);
        assert_eq!(l.window_mapped(102, 900), 1);
        let Some(Event::Finished { how, .. }) = l.take_events().pop() else { panic!("finished") };
        assert_eq!(how, Ended::Window { pid: 102 });
    }

    #[test]
    fn cancel_ends_a_launch_that_did_not_start() {
        let mut l = launches();
        let id = l.begin("Missing", "t", 10);
        l.cancel(id, 12);
        assert!(!l.busy());
        l.cancel(id, 13);
        let events = l.take_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[1], Event::Finished { how: Ended::Cancelled, after_ms: 2, .. }));
    }

    #[test]
    fn events_are_described_for_the_log() {
        let mut l = launches();
        let id = l.begin("Demo Window", "t", 0);
        l.add_pid(id, 100);
        l.window_mapped(101, 840);
        let lines: Vec<String> = l.take_events().iter().flat_map(Event::describe).collect();
        assert_eq!(
            lines,
            [
                "busy pointer on: launching Demo Window (1 pending)",
                "launch Demo Window: window mapped by pid 101 after 840 ms (0 pending)",
                "busy pointer off",
            ]
        );
        l.begin("Slow", "t2", 0);
        l.expire(TIMEOUT_MS);
        let lines: Vec<String> = l.take_events().iter().flat_map(Event::describe).collect();
        assert_eq!(lines[1], "launch Slow: timed out after 30000 ms, no window (0 pending)");
    }

    #[test]
    fn a_pid_is_added_once_and_zero_is_ignored() {
        let mut l = launches();
        let id = l.begin("A", "t", 0);
        l.add_pid(id, 0);
        l.add_pid(id, 100);
        l.add_pid(id, 100);
        assert_eq!(l.list[0].pids, [100]);
        l.add_pid(999, 5);
    }

    #[test]
    fn the_parent_is_read_from_proc_stat() {
        assert_eq!(parent_from_stat("42 (poly-files) S 7 42 42 0 -1 4194304"), Some(7));
        // A command with spaces and parentheses.
        assert_eq!(parent_from_stat("42 (a) b (c) S 9 42"), Some(9));
        assert_eq!(parent_from_stat("42 (x)"), None);
        assert_eq!(parent_from_stat("nonsense"), None);
    }

    #[test]
    fn control_lines_round_trip() {
        let lines = [
            Control::Launch { token: "poly-launch-9-1".into(), name: "Text Editor".into() },
            Control::Pid { token: "poly-launch-9-1".into(), pid: 31 },
            Control::Cancel { token: "poly-launch-9-1".into() },
            Control::Open { id: "poly:Settings".into() },
            Control::Open { id: "org.example.Text Editor.desktop".into() },
        ];
        for c in lines {
            assert_eq!(Control::parse(&c.line()), Some(c));
        }
        assert_eq!(Control::parse("dump"), None);
        assert_eq!(Control::parse("launch onlytoken"), None);
        assert_eq!(Control::parse("pid tok notanumber"), None);
        assert_eq!(Control::parse("launch  bad\u{1}tok name"), None);
        assert_eq!(Control::parse("frobnicate x y"), None);
        assert_eq!(Control::parse("open "), None);
        assert_eq!(Control::parse("open a\u{7}b"), None);
    }
}
