//! In-kernel tests of sockets (ADR 0048): per-socket readiness
//! notification from the network paths, and `/bin/socket-test` from the
//! test initramfs, which makes the socket system calls from ring 3 (TCP
//! between two processes with `poll` and `epoll`, UDP, Unix sockets with
//! descriptor passing, interrupted calls, `SIGPIPE`, netlink).

use crate::fs;
use crate::kernel_test;
use crate::net::file::InetSocket;
use crate::net::{Domain, Protocol, SockType, notify};
use crate::process::ExitStatus;
use crate::{kprintln, program, thread, time};
use alloc::sync::Arc;
use core::net::{Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use poly_vfs::{FileObject, PollEvents, Waker};

/// Counts its wake-ups.
#[derive(Default)]
struct Counter(AtomicUsize);

impl Waker for Counter {
    fn wake(&self, _: PollEvents) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Wait (polling, generously bounded) until `cond` holds.
fn until(mut cond: impl FnMut() -> bool) -> bool {
    let end = time::now_ns() + 30 * time::NANOS_PER_SEC;
    while !cond() {
        if time::now_ns() > end {
            return false;
        }
        thread::sleep(Duration::from_millis(1));
    }
    true
}

kernel_test! {
    fn readiness_changes_wake_only_that_sockets_waiters() -> TestResult {
        let watched = notify::watched();
        let a = InetSocket::new(Domain::Inet, SockType::Datagram, Protocol::Udp).map_err(|_| "socket a")?;
        let b = InetSocket::new(Domain::Inet, SockType::Datagram, Protocol::Udp).map_err(|_| "socket b")?;
        if notify::watched() != watched + 2 {
            return Err("a socket file does not register its queue");
        }
        let lo = |port| SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
        a.with(|s, h| s.bind(h, lo(0))).map_err(|_| "bind a")?;
        b.with(|s, h| s.bind(h, lo(0))).map_err(|_| "bind b")?;
        let pa = a.with(|s, h| s.local_addr(h)).map_err(|_| "name a")?;
        let pb = b.with(|s, h| s.local_addr(h)).map_err(|_| "name b")?;
        let (wa, wb) = (Arc::new(Counter::default()), Arc::new(Counter::default()));
        let _ra = a.queue().register(PollEvents::READABLE, wa.clone());
        let _rb = b.queue().register(PollEvents::READABLE, wb.clone());
        // A datagram for b wakes b's waiters, and not a's.
        let before = wa.0.load(Ordering::SeqCst);
        a.send(b"to b", Some(pb)).map_err(|_| "send to b")?;
        if !until(|| wb.0.load(Ordering::SeqCst) > 0 && b.poll().contains(PollEvents::IN)) {
            return Err("a datagram did not wake the receiver's waiters");
        }
        let mut buf = [0u8; 8];
        if b.recv(&mut buf, false).map(|i| i.len) != Ok(4) {
            return Err("the datagram did not arrive");
        }
        if wa.0.load(Ordering::SeqCst) != before {
            return Err("another socket's datagram woke this socket's waiters");
        }
        b.send(b"to a", Some(pa)).map_err(|_| "send to a")?;
        if !until(|| wa.0.load(Ordering::SeqCst) > before) {
            return Err("the reply did not wake the first socket's waiters");
        }
        drop((a, b));
        if notify::watched() != watched {
            return Err("closed sockets are still watched");
        }
        Ok(())
    }
}

kernel_test! {
    fn socket_test_runs_from_ring_3(env: &TestEnv) -> TestResult {
        if !env.boot.has_flag(fs::TEST_INITRD_FLAG) {
            return Ok(());
        }
        let watched = notify::watched();
        let path: &[u8] = b"/bin/socket-test";
        let process = program::exec_path(path, &[path], &[], false).map_err(|_| "exec_path /bin/socket-test failed")?;
        let status = process
            .wait_until(time::now_ns() + 300 * time::NANOS_PER_SEC)
            .ok_or("/bin/socket-test did not end in time")?;
        if status != ExitStatus::Exited(0) {
            kprintln!("socket test: /bin/socket-test: {status}");
            return Err("/bin/socket-test reported failures");
        }
        // Everything was closed with the processes: no socket is watched
        // any more, no descriptor is in flight.
        if !until(|| notify::watched() == watched) {
            return Err("sockets of ended processes are still watched");
        }
        if super::net::in_flight() != 0 {
            return Err("descriptors are still in flight after every process ended");
        }
        Ok(())
    }
}
