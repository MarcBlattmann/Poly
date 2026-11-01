//! X11 apps through Xwayland (ADR 0118): the X display's sockets, starting
//! the unmodified `Xwayland` server, and its window manager.
//!
//! - **The display.** The first free display number `n` (no live
//!   `/tmp/.X<n>-lock`): the lock file (the pid, ten characters wide, and a
//!   newline), the socket `/tmp/.X11-unix/X<n>` and its twin in the
//!   abstract namespace, `@/tmp/.X11-unix/X<n>` (unix(7)), both made and
//!   listened on here, so X clients can connect before the server runs.
//!   Launched apps get `DISPLAY=:<n>`.
//! - **Starting Xwayland**, at once (`--xwayland`) or when the first X
//!   client connects (`--xwayland-lazy`), and again after it ended:
//!   `Xwayland :<n> -rootless -nolock -shm -listenfd <fs> -listenfd
//!   <abstract> -displayfd <pipe> -wm <socket>` (Xwayland(1)), with its
//!   Wayland connection one end of a socket pair (`WAYLAND_SOCKET`) that
//!   the server knows as Xwayland's (it alone sees `xwayland_shell_v1`).
//!   `-shm`: buffers in shared memory, no GL. When it writes its display
//!   number to the pipe it is ready.
//! - **The window manager** is `poly-x11`'s `Wm` on the other end of the
//!   `-wm` socket pair: what it reads is fed to it, what it queues is
//!   written, its actions go to the server (`x_update`, `x_associate`,
//!   ...), and what the user did to X windows (`take_x_requests`) goes
//!   back to it. Its [`Policy`] places new windows as the scene places
//!   toplevels (dialogs centred over their parent) and reports the frame's
//!   size for `_NET_FRAME_EXTENTS`.

use crate::unix;
use poly_wayland::compose::{BORDER, TITLE_HEIGHT};
use poly_wayland::server::{ClientId, Server, SurfaceRef, XRequest, XWindowState};
use poly_x11::xwm::{self, Action, Policy, Wm};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// The X server.
pub const XWAYLAND: &str = "/usr/bin/Xwayland";

/// When to start Xwayland.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// With the compositor.
    Now,
    /// When the first X client connects.
    Lazy,
}

/// A running Xwayland.
#[derive(Debug)]
struct Running {
    pid: libc::pid_t,
    client: ClientId,
    wm_socket: UnixStream,
    wm: Wm,
    /// The read end of `-displayfd` until Xwayland said it is ready.
    displayfd: Option<std::fs::File>,
}

/// The X display and its server.
#[derive(Debug)]
pub struct Xwayland {
    pub display: u32,
    start: Start,
    lock: PathBuf,
    socket_path: PathBuf,
    listeners: Vec<UnixListener>,
    running: Option<Running>,
    /// Starts so far.
    starts: u32,
}

/// Starts after which a server that keeps ending is given up on.
const MAX_STARTS: u32 = 5;

/// The window manager's questions, answered from the scene.
struct ScenePolicy<'a> {
    server: &'a mut Server,
}

impl Policy for ScenePolicy<'_> {
    fn place(&mut self, w: &xwm::Window, parent: Option<&xwm::Window>) -> (i32, i32) {
        let (width, height) = (w.width as i32, w.height as i32);
        match parent {
            Some(p) => {
                // Centred over the parent, kept in the work area.
                let area = self.server.scene().work_area();
                let x = p.x + (p.width as i32 - width) / 2;
                let y = p.y + (p.height as i32 - height) / 2;
                let top = area.y + TITLE_HEIGHT + BORDER;
                (x.clamp(area.x + BORDER, (area.right() - width - BORDER).max(area.x + BORDER)), y.max(top))
            }
            None => self.server.scene_mut().place(0, 0, width, height, true),
        }
    }

    fn frame_extents(&self) -> [u32; 4] {
        let (b, t) = (BORDER as u32, (TITLE_HEIGHT + BORDER) as u32);
        [b, b, t, b]
    }
}

/// A process with `pid` exists.
fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

/// Take display `n`'s lock file if it is free (or stale).
fn take_lock(n: u32) -> Option<PathBuf> {
    let path = PathBuf::from(format!("/tmp/.X{n}-lock"));
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                // SAFETY: getpid has no preconditions.
                let pid = unsafe { libc::getpid() };
                f.write_all(format!("{pid:>10}\n").as_bytes()).ok()?;
                return Some(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let held = std::fs::read_to_string(&path).ok().and_then(|s| s.trim().parse::<i32>().ok());
                if held.is_some_and(alive) {
                    return None;
                }
                // Stale: whoever held it is gone.
                std::fs::remove_file(&path).ok()?;
            }
            Err(_) => return None,
        }
    }
    None
}

impl Xwayland {
    /// Take the first free display and listen on its sockets.
    pub fn new(start: Start) -> Result<Xwayland, String> {
        let dir = PathBuf::from("/tmp/.X11-unix");
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777));
        }
        let (display, lock) = (0..32).find_map(|n| take_lock(n).map(|l| (n, l))).ok_or("no free X display")?;
        let socket_path = dir.join(format!("X{display}"));
        let _ = std::fs::remove_file(&socket_path);
        let mut listeners =
            vec![UnixListener::bind(&socket_path).map_err(|e| format!("{}: {e}", socket_path.display()))?];
        {
            use std::os::linux::net::SocketAddrExt;
            let name = format!("/tmp/.X11-unix/X{display}");
            match std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())
                .and_then(|a| UnixListener::bind_addr(&a))
            {
                Ok(l) => listeners.push(l),
                Err(e) => eprintln!("poly-compositor: the abstract socket @{name}: {e} (the path alone)"),
            }
        }
        for l in &listeners {
            l.set_nonblocking(true).map_err(|e| format!("X socket: {e}"))?;
        }
        println!(
            "poly-compositor: X display :{display} on {} and {} socket(s){}",
            socket_path.display(),
            listeners.len(),
            if start == Start::Lazy { ", Xwayland starts with the first X client" } else { "" }
        );
        Ok(Xwayland { display, start, lock, socket_path, listeners, running: None, starts: 0 })
    }

    /// `DISPLAY` for launched apps.
    pub fn display_env(&self) -> String {
        format!("DISPLAY=:{}", self.display)
    }

    /// Whether Xwayland should be started now (eagerly, before any client).
    pub fn wants_start(&self) -> bool {
        self.running.is_none() && self.start == Start::Now && self.starts == 0
    }

    /// Start Xwayland: returns its Wayland connection's client ID and our
    /// end of it, for the caller's connection table.
    pub fn start(&mut self, server: &mut Server, env: &[String]) -> Result<(ClientId, UnixStream), String> {
        let (ours, theirs) = UnixStream::pair().map_err(|e| format!("socketpair: {e}"))?;
        let (wm_ours, wm_theirs) = UnixStream::pair().map_err(|e| format!("socketpair: {e}"))?;
        let mut pipe = [0; 2];
        // SAFETY: `pipe` has room for the two descriptors pipe2 writes.
        if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(format!("pipe: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: both descriptors are new and owned by nothing else.
        let (read_end, write_end) = unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) };
        let fd = |f: RawFd| f.to_string();
        let mut argv = vec![
            XWAYLAND.to_string(),
            format!(":{}", self.display),
            "-rootless".into(),
            "-nolock".into(),
            "-shm".into(),
            "-displayfd".into(),
            fd(write_end.as_raw_fd()),
            "-wm".into(),
            fd(wm_theirs.as_raw_fd()),
        ];
        let mut keep = vec![theirs.as_raw_fd(), wm_theirs.as_raw_fd(), write_end.as_raw_fd()];
        for l in &self.listeners {
            argv.push("-listenfd".into());
            argv.push(fd(l.as_raw_fd()));
            keep.push(l.as_raw_fd());
        }
        let mut env: Vec<String> = env.iter().filter(|v| !v.starts_with("DISPLAY=")).cloned().collect();
        env.push(format!("WAYLAND_SOCKET={}", theirs.as_raw_fd()));
        let client = server.add_client();
        server.set_xwayland_client(Some(client));
        let pid = match unix::spawn_keeping(&argv, &env, &keep) {
            Ok(pid) => pid,
            Err(e) => {
                server.remove_client(client);
                return Err(format!("{XWAYLAND}: {e}"));
            }
        };
        self.starts += 1;
        println!("poly-compositor: started Xwayland :{} (pid {pid}): {}", self.display, argv[1..].join(" "));
        let mut wm = Wm::new();
        let mut wm_socket = wm_ours;
        let setup = wm.take_output();
        wm_socket.write_all(&setup).map_err(|e| format!("the window manager's socket: {e}"))?;
        self.running = Some(Running { pid, client, wm_socket, wm, displayfd: Some(std::fs::File::from(read_end)) });
        Ok((client, ours))
    }

    /// The descriptors to poll: the listeners while no server runs (a
    /// connection starts one), the display pipe, the window manager.
    pub fn poll_fds(&self) -> Vec<libc::pollfd> {
        let fd = |fd: RawFd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        match &self.running {
            None if self.starts >= MAX_STARTS => Vec::new(),
            None => self.listeners.iter().map(|l| fd(l.as_raw_fd())).collect(),
            Some(r) => {
                let mut v = vec![fd(r.wm_socket.as_raw_fd())];
                v.extend(r.displayfd.iter().map(|f| fd(f.as_raw_fd())));
                v
            }
        }
    }

    /// After `poll`: start the server for a waiting client, read the
    /// display pipe and the window manager. Returns a new Xwayland
    /// connection for the caller's table.
    pub fn handle(
        &mut self,
        fds: &[libc::pollfd],
        server: &mut Server,
        env: &[String],
    ) -> Option<(ClientId, UnixStream)> {
        let ready = |i: usize| fds.get(i).is_some_and(|p| p.revents != 0);
        let Some(r) = &mut self.running else {
            if (0..fds.len()).any(ready) {
                println!("poly-compositor: an X client connected to :{}", self.display);
                if self.starts + 1 == MAX_STARTS {
                    eprintln!("poly-compositor: Xwayland ended {} times; this is the last start", self.starts);
                }
                match self.start(server, env) {
                    Ok(c) => return Some(c),
                    Err(e) => eprintln!("poly-compositor: {e}"),
                }
            }
            return None;
        };
        if ready(1)
            && let Some(f) = &mut r.displayfd
        {
            let mut buf = [0u8; 32];
            match f.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let text = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                    println!("poly-compositor: Xwayland ready on :{text}");
                    r.displayfd = None;
                }
                _ => r.displayfd = None,
            }
        }
        if ready(0) {
            let mut buf = vec![0u8; 65536];
            match r.wm_socket.read(&mut buf) {
                Ok(0) | Err(_) => {
                    eprintln!("poly-compositor: the X window manager's connection closed");
                    r.displayfd = None;
                    // Xwayland ends too; reap() notices.
                }
                Ok(n) => {
                    let mut policy = ScenePolicy { server };
                    if let Err(e) = r.wm.feed(&buf[..n], &mut policy) {
                        eprintln!("poly-compositor: the X window manager: {}", e.0);
                    }
                }
            }
        }
        self.apply(server);
        None
    }

    /// The window manager's actions to the server, the user's requests to
    /// the window manager, its requests to Xwayland.
    pub fn pump(&mut self, server: &mut Server) {
        self.apply(server);
    }

    fn apply(&mut self, server: &mut Server) {
        let Some(r) = &mut self.running else { return };
        // POLY_XWM_DEBUG: every action and request, for debugging.
        let debug = std::env::var_os("POLY_XWM_DEBUG").is_some();
        for _ in 0..4 {
            let actions = r.wm.take_actions();
            for a in &actions {
                if debug {
                    match a {
                        Action::Changed(id) => println!("poly-compositor: xwm: {a:?} {:?}", r.wm.window(*id)),
                        _ => println!("poly-compositor: xwm: {a:?}"),
                    }
                }
                match a {
                    Action::Ready => println!("poly-compositor: X window manager ready"),
                    Action::Changed(id) => {
                        if let Some(w) = r.wm.window(*id) {
                            let state = XWindowState {
                                x: w.x,
                                y: w.y,
                                width: w.width,
                                height: w.height,
                                mapped: w.mapped,
                                managed: w.managed,
                                override_redirect: w.override_redirect,
                                decorated: w.decorated(r.wm.atoms()),
                                title: w.title().to_string(),
                                transient_for: w.transient_for,
                            };
                            server.x_update(*id, &state);
                        }
                    }
                    Action::Destroyed(id) => server.x_destroy(*id),
                    Action::Associate { window, surface } => {
                        let by = match surface {
                            xwm::SurfaceRef::Serial(s) => SurfaceRef::Serial(*s),
                            xwm::SurfaceRef::Id(i) => SurfaceRef::Id(*i),
                        };
                        server.x_associate(*window, by);
                    }
                    Action::Activate(id) => server.x_activate(*id),
                    Action::Move(id) => server.x_start_move(*id),
                    Action::Log(s) => eprintln!("poly-compositor: X window manager: {s}"),
                }
            }
            let requests = server.take_x_requests();
            for q in &requests {
                if debug {
                    println!("poly-compositor: xwm: user {q:?}");
                }
                match *q {
                    XRequest::Activate(w) => r.wm.activate(Some(w)),
                    XRequest::Deactivate => r.wm.activate(None),
                    XRequest::Close(w) => r.wm.close(w),
                    XRequest::Move { window, x, y } => r.wm.move_to(window, x, y),
                }
            }
            if actions.is_empty() && requests.is_empty() {
                break;
            }
        }
        let out = r.wm.take_output();
        if !out.is_empty()
            && let Err(e) = r.wm_socket.write_all(&out)
        {
            eprintln!("poly-compositor: the X window manager's socket: {e}");
        }
    }

    /// If Xwayland ended: forget it (the next X client starts another).
    /// Returns its connection, for the caller to drop.
    pub fn reap(&mut self, server: &mut Server) -> Option<ClientId> {
        let r = self.running.as_ref()?;
        let status = unix::try_wait(r.pid)?;
        let r = self.running.take()?;
        println!("poly-compositor: Xwayland (pid {}) exited with status {status}", r.pid);
        server.set_xwayland_client(None);
        if self.start == Start::Now {
            self.start = Start::Lazy;
        }
        Some(r.client)
    }
}

impl Drop for Xwayland {
    fn drop(&mut self) {
        if let Some(r) = &self.running {
            // SAFETY: signalling our own child.
            unsafe { libc::kill(r.pid, libc::SIGTERM) };
        }
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.lock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_wayland::server::{Config, OutputConfig};

    #[test]
    fn the_policy_places_like_the_scene_and_centres_dialogs() {
        let output = OutputConfig { width: 800, height: 600, ..OutputConfig::default() };
        let mut server = Server::new(Config { output, ..Config::default() }, Box::new(unix::UnixHost::default()));
        server.scene_mut().set_placement(poly_wayland::compose::Placement::Centered);
        server.scene_mut().set_reserved_top(24);
        let mut p = ScenePolicy { server: &mut server };
        let window = |x: i32, y: i32, width: u32, height: u32| {
            let mut w = xwm::Window::default();
            (w.x, w.y, w.width, w.height) = (x, y, width, height);
            w
        };
        let w = window(0, 0, 200, 100);
        let (x, y) = p.place(&w, None);
        // Centred in the work area (800 x 576 below the ridge), frame included.
        assert_eq!(x, (800 - 202) / 2 + 1);
        assert_eq!(y, 24 + (576 - 126) / 2 + 25);
        let parent = window(100, 200, 400, 300);
        assert_eq!(p.place(&w, Some(&parent)), (200, 300));
        // A parent at the edge: the dialog stays in the work area.
        let parent = window(-300, 0, 100, 50);
        assert_eq!(p.place(&w, Some(&parent)), (1, 24 + 25));
        assert_eq!(p.frame_extents(), [1, 1, 25, 1]);
    }

    #[test]
    fn locks_are_taken_once_and_stale_ones_replaced() {
        // A display number no real server here uses.
        let n = 900 + std::process::id() % 50;
        let path = PathBuf::from(format!("/tmp/.X{n}-lock"));
        let _ = std::fs::remove_file(&path);
        let got = take_lock(n).expect("free");
        let text = std::fs::read_to_string(&got).unwrap();
        assert_eq!(text.len(), 11);
        assert_eq!(text.trim().parse::<u32>().unwrap(), std::process::id());
        assert!(take_lock(n).is_none(), "held by a live process (us)");
        std::fs::write(&path, format!("{:>10}\n", i32::MAX)).unwrap();
        assert!(take_lock(n).is_some(), "stale");
        let _ = std::fs::remove_file(&path);
    }
}
