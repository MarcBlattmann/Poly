//! `poly-compositor`: Poly's Wayland compositor for the Linux personality
//! (ADR 0106).
//!
//! It listens on `$XDG_RUNTIME_DIR/<socket>` (`wayland-0` by default) as
//! the Wayland book's "Wayland display" describes, hands every client's
//! bytes and file descriptors to `poly-wayland`'s server, and composes the
//! scene into an off-screen framebuffer at up to 60 frames a second (each
//! frame answers the frame callbacks committed before it). Until the
//! virtio-gpu driver's interface is there, the picture goes nowhere but a
//! PPM file (`--dump`, else `$XDG_RUNTIME_DIR/poly-compositor.ppm`):
//! written on `SIGUSR1`, with the windows and the `--probe` pixels printed,
//! and with `--dump` when the compositor ends.
//!
//! ```text
//! poly-compositor [--socket NAME] [--size WxH] [--scale N] [--dump FILE]
//!                 [--probe X,Y]... [--exec PROGRAM [ARGS...]]
//! ```
//!
//! `--exec` starts a client once the socket is up (with `WAYLAND_DISPLAY`,
//! `XDG_RUNTIME_DIR` and `POLY_COMPOSITOR_DUMP`, the dump's path, set) and
//! ends the compositor when it ends, with its exit status: how the boot
//! test runs it as Poly's first process, the client asking for a dump
//! with `SIGUSR1` while its window is up. Without `XDG_RUNTIME_DIR` it
//! uses `/run/user/<uid>`, made if missing.
//!
//! I/O is blocking and driven by `poll`: one `recvmsg` per readable
//! client, `sendmsg` of what the server queued. A client that stops
//! reading could stall it; non-blocking sends come with real workloads.

mod unix;

use poly_wayland::compose::{Framebuffer as _, MemoryFramebuffer};
use poly_wayland::server::{ClientId, Config, FrameTime, OutputConfig, Server};
use poly_wayland::wire::Fd;
use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

/// The shortest time between two frames: 60 Hz.
const FRAME_NS: u64 = 1_000_000_000 / 60;

/// Set by `SIGUSR1`: write the picture.
static DUMP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigusr1(_: libc::c_int) {
    DUMP.store(true, Ordering::Relaxed);
}

#[derive(Debug)]
struct Options {
    socket: String,
    width: u32,
    height: u32,
    scale: u32,
    dump: Option<PathBuf>,
    probes: Vec<(u32, u32)>,
    exec: Vec<String>,
}

const USAGE: &str = "usage: poly-compositor [--socket NAME] [--size WxH] [--scale N] [--dump FILE] [--probe X,Y]... [--exec PROGRAM [ARGS...]]";

fn pair(s: &str, sep: char) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(sep)?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        socket: "wayland-0".into(),
        width: 1024,
        height: 768,
        scale: 1,
        dump: None,
        probes: Vec::new(),
        exec: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value\n{USAGE}"));
        match a.as_str() {
            "--socket" => o.socket = value()?,
            "--size" => {
                let v = value()?;
                (o.width, o.height) = pair(&v, 'x').filter(|(w, h)| *w > 0 && *h > 0).ok_or(format!("bad size {v}"))?;
            }
            "--scale" => {
                let v = value()?;
                o.scale = v.parse().ok().filter(|s| (1..=8).contains(s)).ok_or(format!("bad scale {v}"))?;
            }
            "--dump" => o.dump = Some(value()?.into()),
            "--probe" => {
                let v = value()?;
                o.probes.push(pair(&v, ',').ok_or(format!("bad probe {v}"))?);
            }
            "--exec" => {
                o.exec = it.by_ref().cloned().collect();
                if o.exec.is_empty() {
                    return Err(format!("--exec needs a program\n{USAGE}"));
                }
            }
            "--help" | "-h" => return Err(USAGE.into()),
            _ => return Err(format!("unknown argument {a}\n{USAGE}")),
        }
    }
    Ok(o)
}

/// `$XDG_RUNTIME_DIR`, or `/run/user/<uid>` made with mode 0700.
fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(dir.into());
    }
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/run/user/{uid}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    Ok(dir)
}

/// A connected client: its socket and what is still to be sent.
#[derive(Debug)]
struct Connection {
    stream: UnixStream,
    out: Vec<u8>,
    out_fds: Vec<Fd>,
}

/// Send what is queued; false if the connection is broken.
fn flush(conn: &mut Connection) -> bool {
    while !conn.out.is_empty() {
        match unix::send(conn.stream.as_raw_fd(), &conn.out, &conn.out_fds) {
            Ok(n) => {
                conn.out.drain(..n);
                let sent = conn.out_fds.len().min(unix::MAX_FDS);
                for fd in conn.out_fds.drain(..sent) {
                    unix::close(fd.0);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

/// Write the picture as a PPM: to a temporary file renamed into place, so
/// a reader never sees half of it.
fn write_dump(fb: &MemoryFramebuffer, path: &Path) {
    let tmp = path.with_extension("ppm.tmp");
    match std::fs::write(&tmp, fb.to_ppm()).and_then(|()| std::fs::rename(&tmp, path)) {
        Ok(()) => println!("poly-compositor: wrote {} ({}x{})", path.display(), fb.width(), fb.height()),
        Err(e) => eprintln!("poly-compositor: {}: {e}", path.display()),
    }
}

/// Draw, describe the windows and the probed pixels, write the picture:
/// what `SIGUSR1` asks for.
fn report(server: &mut Server, fb: &mut MemoryFramebuffer, probes: &[(u32, u32)], path: &Path) {
    server.render(fb);
    for w in server.scene().windows() {
        let (x, y) = w.position();
        let (cw, ch) = w.content().logical_size();
        println!("poly-compositor: window {:?} at {x},{y} size {cw}x{ch}", w.title());
    }
    for &(x, y) in probes {
        match fb.pixel(x, y) {
            Some(p) => println!("poly-compositor: pixel {x},{y} = #{p:06x}"),
            None => println!("poly-compositor: pixel {x},{y} is outside {}x{}", fb.width(), fb.height()),
        }
    }
    write_dump(fb, path);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("poly-compositor: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<u8, String> {
    let o = parse_args(args)?;
    let dir = runtime_dir()?;
    let path = dir.join(&o.socket);
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dump_path = o.dump.clone().unwrap_or_else(|| dir.join("poly-compositor.ppm"));
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe { libc::signal(libc::SIGUSR1, on_sigusr1 as *const () as libc::sighandler_t) };

    let output = OutputConfig { width: o.width, height: o.height, scale: o.scale, ..OutputConfig::default() };
    let mut server = Server::new(Config { output, ..Config::default() }, Box::new(unix::UnixHost));
    let mut fb = MemoryFramebuffer::new(o.width, o.height);
    println!("poly-compositor: listening on {} ({}x{}, scale {})", path.display(), o.width, o.height, o.scale);

    let child = if o.exec.is_empty() {
        None
    } else {
        let mut env: Vec<String> = std::env::vars()
            .filter(|(k, _)| !["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "POLY_COMPOSITOR_DUMP"].contains(&k.as_str()))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        env.push(format!("WAYLAND_DISPLAY={}", o.socket));
        env.push(format!("XDG_RUNTIME_DIR={}", dir.display()));
        // Where SIGUSR1 puts the picture, for a client that checks it.
        env.push(format!("POLY_COMPOSITOR_DUMP={}", dump_path.display()));
        let pid = unix::spawn(&o.exec, &env).map_err(|e| format!("{}: {e}", o.exec[0]))?;
        println!("poly-compositor: started {} (pid {pid})", o.exec[0]);
        Some(pid)
    };

    let mut conns: BTreeMap<ClientId, Connection> = BTreeMap::new();
    let mut next_frame = 0u64;
    let mut buf = vec![0u8; 4096];
    loop {
        // What to wait for, and how long.
        let mut fds = vec![libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        let ids: Vec<ClientId> = conns.keys().copied().collect();
        fds.extend(ids.iter().map(|id| libc::pollfd {
            fd: conns[id].stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }));
        let now = unix::monotonic_ns();
        let timeout = if server.wants_frame() {
            (next_frame.saturating_sub(now) / 1_000_000) as i32
        } else if child.is_some() {
            50
        } else {
            -1
        };
        unix::poll(&mut fds, timeout).map_err(|e| format!("poll: {e}"))?;

        if fds[0].revents & libc::POLLIN != 0 {
            match listener.accept() {
                Ok((stream, _)) => {
                    let id = server.add_client();
                    conns.insert(id, Connection { stream, out: Vec::new(), out_fds: Vec::new() });
                }
                Err(e) => eprintln!("poly-compositor: accept: {e}"),
            }
        }
        let mut gone = Vec::new();
        for (pfd, id) in fds[1..].iter().zip(&ids) {
            if pfd.revents == 0 {
                continue;
            }
            let mut received = Vec::new();
            match unix::recv(pfd.fd, &mut buf, &mut received) {
                Ok(0) => gone.push(*id),
                Ok(n) => server.receive(*id, &buf[..n], received),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => gone.push(*id),
            }
        }

        // A frame when one is due.
        let now = unix::monotonic_ns();
        if server.wants_frame() && now >= next_frame {
            server.render(&mut fb);
            server.frame_presented(FrameTime { monotonic_ns: now });
            next_frame = now + FRAME_NS;
        }

        // Send what the server queued; cut off failed clients.
        for (id, conn) in conns.iter_mut() {
            let (bytes, out_fds) = server.take_output(*id);
            conn.out.extend_from_slice(&bytes);
            conn.out_fds.extend(out_fds);
            if !flush(conn) {
                gone.push(*id);
            }
            if let Some(why) = server.failure(*id) {
                eprintln!("poly-compositor: client {} cut off: {why}", id.0);
                gone.push(*id);
            }
        }
        gone.sort_unstable();
        gone.dedup();
        for id in gone {
            server.remove_client(id);
            conns.remove(&id);
        }

        if DUMP.swap(false, Ordering::Relaxed) {
            report(&mut server, &mut fb, &o.probes, &dump_path);
        }

        if let Some(status) = child.and_then(unix::try_wait) {
            if o.dump.is_some() {
                server.render(&mut fb);
                write_dump(&fb, &dump_path);
            }
            println!("poly-compositor: client exited with status {status}");
            let _ = std::fs::remove_file(&path);
            return Ok(status.clamp(0, 255) as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_its_arguments() {
        let o = parse_args(&args("--size 640x480 --scale 2 --probe 1,2 --probe 3,4 --exec /bin/c -x")).unwrap();
        assert_eq!((o.width, o.height, o.scale), (640, 480, 2));
        assert_eq!(o.probes, [(1, 2), (3, 4)]);
        assert_eq!(o.exec, ["/bin/c", "-x"]);
        assert_eq!(o.socket, "wayland-0");
        assert!(parse_args(&args("--size 0x4")).is_err());
        assert!(parse_args(&args("--scale 9")).is_err());
        assert!(parse_args(&args("--exec")).is_err());
        assert!(parse_args(&args("--what")).is_err());
    }

    /// The compositor's own transport: a client's bytes and a file
    /// descriptor through a socket pair, its shared memory mapped.
    #[test]
    fn passes_file_descriptors_and_maps_shared_memory() {
        let (a, b) = UnixStream::pair().unwrap();
        let path = std::env::temp_dir().join(format!("poly-compositor-test-{}", std::process::id()));
        std::fs::write(&path, [1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(unix::send(a.as_raw_fd(), b"hello", &[Fd(file.as_raw_fd())]).unwrap(), 5);
        let (mut buf, mut fds) = ([0u8; 16], Vec::new());
        assert_eq!(unix::recv(b.as_raw_fd(), &mut buf, &mut fds).unwrap(), 5);
        assert_eq!((&buf[..5], fds.len()), (&b"hello"[..], 1));
        use poly_wayland::server::Host;
        let mut map = unix::UnixHost.map_shm(fds[0], 8).unwrap();
        let mut out = [0u8; 4];
        assert!(map.read(4, &mut out));
        assert_eq!(out, [5, 6, 7, 8]);
        assert!(!map.read(6, &mut out));
        assert!(map.resize(8));
    }
}
