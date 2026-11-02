//! The session bus the compositor hosts (ADR 0117): `poly-dbus`'s bus on a
//! Unix socket at `$XDG_RUNTIME_DIR/bus` (the address clients find in
//! `DBUS_SESSION_BUS_ADDRESS`, and where GLib looks without it), with the
//! shell's `org.freedesktop.Notifications` as a local service. Linux
//! clients (the toolkits) connect to the path through the Linux
//! personality's sockets, the same objects (ADR 0125).

use crate::io::{self, Read, Write};
use crate::os::{self, RawFd};
use crate::prelude::*;
use alloc::collections::BTreeMap;
use poly_dbus::Message;
use poly_dbus::bus::{Bus, ConnId};
use poly_rt::fs;
use poly_rt::net::{UnixListener, UnixStream};

/// The bus, its socket and its connections.
#[derive(Debug)]
pub struct SessionBus {
    listener: UnixListener,
    path: String,
    pub bus: Bus,
    conns: BTreeMap<ConnId, UnixStream>,
}

/// 16 random bytes as 32 hexadecimal digits: the bus's GUID.
fn guid() -> String {
    let mut b = [0u8; 16];
    if fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_err() {
        // A GUID only has to differ between buses; the time will do.
        let t = os::monotonic_ns() ^ u64::from(os::getpid()) << 32;
        b[..8].copy_from_slice(&t.to_le_bytes());
        b[8..].copy_from_slice(&t.rotate_left(17).to_le_bytes());
    }
    poly_dbus::auth::hex_encode(&b)
}

/// The peer's user ID from the socket, if it tells.
fn peer_uid(s: &UnixStream) -> Option<u32> {
    s.peer().ok().filter(|p| p.pid != 0).map(|p| p.uid)
}

impl SessionBus {
    /// Listen on `dir/bus`; the local services own `names`.
    pub fn open(dir: &str, names: &[&str]) -> io::Result<SessionBus> {
        let path = fs::join(dir, "bus");
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let mut bus = Bus::new(&guid());
        for n in names {
            bus.request_local_name(n);
        }
        Ok(SessionBus { listener, path, bus, conns: BTreeMap::new() })
    }

    /// The address for `DBUS_SESSION_BUS_ADDRESS`.
    pub fn address(&self) -> String {
        format!("unix:path={}", self.path)
    }

    /// The listener's descriptor and each connection's, to poll.
    pub fn poll_fds(&self) -> Vec<(RawFd, Option<ConnId>)> {
        let mut v = vec![(self.listener.handle() as RawFd, None)];
        v.extend(self.conns.iter().map(|(id, s)| (s.handle() as RawFd, Some(*id))));
        v
    }

    /// The listener is readable: a new connection.
    pub fn accept(&mut self) {
        if let Ok(s) = self.listener.accept() {
            let id = self.bus.connect(peer_uid(&s));
            self.conns.insert(id, s);
        }
    }

    /// A connection is readable.
    pub fn readable(&mut self, id: ConnId) {
        let mut buf = [0u8; 8192];
        let n = match self.conns.get_mut(&id).map(|s| s.read(&mut buf)) {
            Some(Ok(n)) => n,
            Some(Err(io::ErrorKind::Interrupted)) => return,
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
        let _ = fs::remove_file(&self.path);
    }
}
