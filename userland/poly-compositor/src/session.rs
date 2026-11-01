//! The session bus the compositor hosts (ADR 0117): `poly-dbus`'s bus on a
//! Unix socket at `$XDG_RUNTIME_DIR/bus` (the address clients find in
//! `DBUS_SESSION_BUS_ADDRESS`, and where GLib looks without it), with the
//! shell's `org.freedesktop.Notifications` as a local service.

use poly_dbus::Message;
use poly_dbus::bus::{Bus, ConnId};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The bus, its socket and its connections.
#[derive(Debug)]
pub struct SessionBus {
    listener: UnixListener,
    path: PathBuf,
    pub bus: Bus,
    conns: BTreeMap<ConnId, UnixStream>,
}

/// 16 random bytes as 32 hexadecimal digits: the bus's GUID.
fn guid() -> String {
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // A GUID only has to differ between buses; the time will do.
        let t = crate::unix::monotonic_ns() ^ u64::from(std::process::id()) << 32;
        b[..8].copy_from_slice(&t.to_le_bytes());
        b[8..].copy_from_slice(&t.rotate_left(17).to_le_bytes());
    }
    poly_dbus::auth::hex_encode(&b)
}

/// The peer's user ID from the socket (`SO_PEERCRED`), if it tells.
fn peer_uid(s: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for getsockopt to fill.
    let r = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (r == 0 && len as usize == std::mem::size_of::<libc::ucred>() && cred.pid != 0).then_some(cred.uid)
}

impl SessionBus {
    /// Listen on `dir/bus`; the local services own `names`.
    pub fn open(dir: &Path, names: &[&str]) -> std::io::Result<SessionBus> {
        let path = dir.join("bus");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let mut bus = Bus::new(&guid());
        for n in names {
            bus.request_local_name(n);
        }
        Ok(SessionBus { listener, path, bus, conns: BTreeMap::new() })
    }

    /// The address for `DBUS_SESSION_BUS_ADDRESS`.
    pub fn address(&self) -> String {
        format!("unix:path={}", self.path.display())
    }

    /// The listener's descriptor and each connection's, to poll.
    pub fn poll_fds(&self) -> Vec<(RawFd, Option<ConnId>)> {
        let mut v = vec![(self.listener.as_raw_fd(), None)];
        v.extend(self.conns.iter().map(|(id, s)| (s.as_raw_fd(), Some(*id))));
        v
    }

    /// The listener is readable: a new connection.
    pub fn accept(&mut self) {
        if let Ok((s, _)) = self.listener.accept() {
            let id = self.bus.connect(peer_uid(&s));
            self.conns.insert(id, s);
        }
    }

    /// A connection is readable.
    pub fn readable(&mut self, id: ConnId) {
        let mut buf = [0u8; 8192];
        let n = match self.conns.get_mut(&id).map(|s| s.read(&mut buf)) {
            Some(Ok(n)) => n,
            Some(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => return,
            _ => 0,
        };
        if n == 0 {
            self.drop_conn(id);
        } else {
            self.bus.receive(id, &buf[..n]);
        }
    }

    fn drop_conn(&mut self, id: ConnId) {
        self.conns.remove(&id);
        self.bus.disconnect(id);
    }

    /// Method calls to the local services.
    pub fn take_local(&mut self) -> Vec<Message> {
        self.bus.take_local()
    }

    pub fn send_local(&mut self, m: Message) {
        self.bus.send_local(m);
    }

    /// Send what is queued; close failed connections.
    pub fn flush(&mut self) {
        let ids: Vec<ConnId> = self.conns.keys().copied().collect();
        for id in ids {
            let out = self.bus.take_output(id);
            let failed = self.bus.failure(id).map(String::from);
            let broken = !out.is_empty() && self.conns.get_mut(&id).is_some_and(|s| s.write_all(&out).is_err());
            if let Some(why) = &failed {
                eprintln!("poly-compositor: bus connection {} cut off: {why}", id.0);
            }
            if broken || failed.is_some() {
                self.drop_conn(id);
            }
        }
    }
}

impl Drop for SessionBus {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use poly_dbus::message::Stream;
    use poly_dbus::notifications;

    /// A client speaking to the bus over a real socket: authenticate,
    /// Hello, Notify; the local service answers.
    #[test]
    fn a_client_notifies_over_the_socket() {
        let dir = std::env::temp_dir().join(format!("poly-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut sb = SessionBus::open(&dir, &[notifications::NAME]).unwrap();
        assert!(sb.address().ends_with("/bus"));
        let mut c = UnixStream::connect(dir.join("bus")).unwrap();
        sb.accept();
        let (auth, begin) = poly_dbus::auth::client_lines(0);
        c.write_all(&auth).unwrap();
        let id = sb.poll_fds()[1].1.unwrap();
        sb.readable(id);
        sb.flush();
        let mut buf = [0u8; 256];
        let n = c.read(&mut buf).unwrap();
        assert!(buf[..n].starts_with(b"OK "));
        c.write_all(&begin).unwrap();
        let hello = Message::method_call(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "Hello",
            vec![],
        );
        c.write_all(&hello.encode(1)).unwrap();
        let n = notifications::Notify {
            app_name: "t".into(),
            summary: "Hi".into(),
            expire_timeout: -1,
            ..Default::default()
        };
        c.write_all(&notifications::notify_call(&n).encode(2)).unwrap();
        // Both may arrive in one read or two.
        let mut local = Vec::new();
        for _ in 0..4 {
            sb.readable(id);
            local.extend(sb.take_local());
            if !local.is_empty() {
                break;
            }
        }
        assert_eq!(local.len(), 1);
        assert_eq!(notifications::parse(&local[0]), Ok(notifications::Request::Notify(n)));
        sb.send_local(notifications::notify_reply(&local[0], 1));
        sb.flush();
        let mut stream = Stream::default();
        let mut got = Vec::new();
        while got.iter().all(|m: &Message| m.reply_serial != Some(2)) {
            let n = c.read(&mut buf).unwrap();
            stream.push(&buf[..n]);
            while let Some(m) = stream.next_message() {
                got.push(m.unwrap());
            }
        }
        let reply = got.iter().find(|m| m.reply_serial == Some(2)).unwrap();
        assert_eq!(reply.body, [poly_dbus::Value::Uint32(1)]);
        drop(sb);
        assert!(!dir.join("bus").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
