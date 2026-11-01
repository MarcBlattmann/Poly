//! Host tests of Unix domain sockets, against unix(7), socket(7) and the
//! socket calls' manual pages, on a real VFS over tmpfs.

extern crate std;

use crate::*;
use alloc::vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use poly_vfs::poll::Waker;
use poly_vfs::{Clock, OpenFlags, Timespec, Tmpfs};

struct ZeroClock;

impl Clock for ZeroClock {
    fn now(&self) -> Timespec {
        Timespec::default()
    }
}

struct Env {
    vfs: Vfs,
    ctx: FsContext,
    root: Cred,
    ns: Arc<Namespace>,
}

fn env() -> Env {
    let vfs = Vfs::new(Arc::new(Tmpfs::new(Arc::new(ZeroClock))));
    let ctx = vfs.root_context().unwrap();
    Env { vfs, ctx, root: Cred::root(), ns: Namespace::new() }
}

const ME: Credentials = Credentials { pid: 10, uid: 0, gid: 0 };
const OTHER: Credentials = Credentials { pid: 20, uid: 1000, gid: 100 };

fn recv(s: &UnixSocket, n: usize) -> Result<(Vec<u8>, Received)> {
    let mut buf = vec![0u8; n];
    let r = s.recv(&mut buf, false)?;
    buf.truncate(r.len);
    Ok((buf, r))
}

fn send(s: &Arc<UnixSocket>, data: &[u8]) -> Result<usize> {
    s.send(data, None, Ancillary::default(), ME)
}

/// An open file on `object`, as the kernel makes one for a descriptor.
fn file(e: &Env, object: Arc<dyn FileObject>) -> Arc<OpenFile> {
    let node = e.vfs.anon_node(FileType::Socket, 0o777, &e.root).unwrap();
    Arc::new(OpenFile::on_object(node, object, OpenFlags::RDWR))
}

/// Counts its wake-ups.
#[derive(Default)]
struct Counter(AtomicUsize);

impl Waker for Counter {
    fn wake(&self, _: PollEvents) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn socketpair_streams_bytes_both_ways_then_eof_and_epipe() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    assert!(a.poll().contains(PollEvents::WRITABLE));
    assert!(!a.poll().intersects(PollEvents::READABLE));
    assert_eq!(recv(&a, 10).err(), Some(Errno::EAGAIN));
    assert_eq!(send(&a, b"hello"), Ok(5));
    assert_eq!(send(&a, b" world"), Ok(6));
    assert!(b.poll().contains(PollEvents::READABLE));
    assert_eq!(b.bytes_readable(), Ok(11));
    // A stream receive crosses packets.
    let (got, r) = recv(&b, 100).unwrap();
    assert_eq!((got.as_slice(), r.full_len), (b"hello world".as_slice(), 11));
    assert_eq!(send(&b, b"back"), Ok(4));
    assert_eq!(recv(&a, 2).unwrap().0, b"ba");
    assert_eq!(recv(&a, 10).unwrap().0, b"ck");
    // socketpair(2): each end's peer credentials are the creator's.
    assert_eq!(a.peer_credentials(), Some(ME));
    assert_eq!((a.local_name(), b.peer_name()), (Address::Unnamed, Ok(Address::Unnamed)));
    // The peer closes: end of file, then EPIPE for writes.
    b.close();
    assert!(a.poll().contains(PollEvents::READABLE | PollEvents::HUP | PollEvents::RDHUP));
    assert_eq!(recv(&a, 10).unwrap().1.len, 0);
    assert_eq!(send(&a, b"x"), Err(Errno::EPIPE));
}

#[test]
fn closing_with_unread_data_resets_the_peer() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    send(&a, b"for b").unwrap();
    send(&b, b"for a").unwrap();
    b.close();
    // a reads what was queued for it, then the reset, then end of file.
    assert_eq!(recv(&a, 10).unwrap().0, b"for a");
    assert_eq!(recv(&a, 10).err(), Some(Errno::ECONNRESET));
    assert_eq!(recv(&a, 10).unwrap().1.len, 0);
}

#[test]
fn stream_writes_take_what_fits_and_wake_the_writer() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    a.set_buffers(Some(4096), None);
    let big = vec![7u8; 10_000];
    assert_eq!(send(&a, &big), Ok(4096));
    assert_eq!(send(&a, b"x"), Err(Errno::EAGAIN));
    assert!(!a.poll().intersects(PollEvents::WRITABLE));
    assert_eq!(a.send_queued(), 4096);
    let w = Arc::new(Counter::default());
    let _reg = a.wait_queue().register(PollEvents::WRITABLE, w.clone());
    assert_eq!(recv(&b, 1000).unwrap().1.len, 1000);
    assert!(w.0.load(Ordering::SeqCst) >= 1, "reading woke the writer");
    assert!(a.poll().contains(PollEvents::WRITABLE));
}

#[test]
fn shutdown_directions() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    send(&a, b"last").unwrap();
    a.shutdown(Shutdown::Write).unwrap();
    assert_eq!(send(&a, b"more"), Err(Errno::EPIPE));
    assert_eq!(recv(&b, 10).unwrap().0, b"last");
    assert_eq!(recv(&b, 10).unwrap().1.len, 0, "end of file after SHUT_WR");
    assert!(b.poll().contains(PollEvents::RDHUP));
    b.shutdown(Shutdown::Read).unwrap();
    // b's own writes still go; a's would fail.
    assert_eq!(send(&b, b"ok"), Ok(2));
    let fresh = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(fresh.shutdown(Shutdown::Both), Ok(()), "an unconnected socket accepts it");
}

#[test]
fn ancillary_data_is_a_barrier_for_stream_receives() {
    // unix(7)'s example: four bytes, one byte with ancillary data, four
    // bytes; receives of 20 bytes get five bytes and the ancillary data,
    // then four.
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    let (r, _w) = poly_vfs::pipe::Pipe::pair(false);
    let passed = file(&e, Arc::new(r));
    send(&a, b"1234").unwrap();
    a.send(b"5", None, Ancillary { rights: vec![passed.clone()], creds: None }, ME).unwrap();
    send(&a, b"6789").unwrap();
    assert_eq!(e.ns.in_flight(), 1);
    let (got, r1) = recv(&b, 20).unwrap();
    assert_eq!(got, b"12345");
    assert_eq!(r1.rights.len(), 1);
    assert!(Arc::ptr_eq(&r1.rights[0], &passed));
    assert_eq!(e.ns.in_flight(), 0, "received");
    let (got, r2) = recv(&b, 20).unwrap();
    assert_eq!((got.as_slice(), r2.rights.len()), (b"6789".as_slice(), 0));
}

#[test]
fn credentials_with_so_passcred() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    send(&a, b"x").unwrap();
    assert_eq!(recv(&b, 4).unwrap().1.creds, None, "only with SO_PASSCRED");
    b.set_passcred(true);
    send(&a, b"ab").unwrap();
    a.send(b"cd", None, Ancillary { rights: Vec::new(), creds: Some(OTHER) }, ME).unwrap();
    // Different credentials are not merged into one receive.
    let (got, r) = recv(&b, 10).unwrap();
    assert_eq!((got.as_slice(), r.creds), (b"ab".as_slice(), Some(ME)));
    let (got, r) = recv(&b, 10).unwrap();
    assert_eq!((got.as_slice(), r.creds), (b"cd".as_slice(), Some(OTHER)));
}

#[test]
fn listen_connect_accept_on_abstract_names() {
    let e = env();
    let l = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    l.bind_abstract(b"svc").unwrap();
    let other = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(other.bind_abstract(b"svc"), Err(Errno::EADDRINUSE));
    assert_eq!(l.bind_abstract(b"again"), Err(Errno::EINVAL), "named already");
    let c = UnixSocket::new(&e.ns, SocketType::Stream, OTHER);
    let target = e.ns.lookup_abstract(b"svc").unwrap();
    assert_eq!(c.connect(&target, OTHER), Err(Errno::ECONNREFUSED), "not listening");
    l.listen(0, ME).unwrap();
    assert!(l.is_listening());
    assert_eq!(l.accept().err(), Some(Errno::EAGAIN));
    c.connect(&target, OTHER).unwrap();
    assert!(l.poll().contains(PollEvents::READABLE));
    assert_eq!(c.connect(&target, OTHER), Err(Errno::EISCONN));
    // The queue (backlog 0 + 1) is full: a second client waits, and is
    // woken by accept.
    let c2 = UnixSocket::new(&e.ns, SocketType::Stream, OTHER);
    assert_eq!(c2.connect(&target, OTHER), Err(Errno::EAGAIN));
    let w = Arc::new(Counter::default());
    let _reg = c2.wait_queue().register(PollEvents::WRITABLE, w.clone());
    let s = l.accept().unwrap();
    assert!(w.0.load(Ordering::SeqCst) >= 1);
    c2.connect(&target, OTHER).unwrap();
    // Names and credentials.
    assert_eq!(s.local_name(), Address::Abstract(b"svc".to_vec()));
    assert_eq!(c.peer_name(), Ok(Address::Abstract(b"svc".to_vec())));
    assert_eq!(s.peer_name(), Ok(Address::Unnamed));
    assert_eq!(s.peer_credentials(), Some(OTHER));
    assert_eq!(c.peer_credentials(), Some(ME));
    // Data both ways.
    c.send(b"ping", None, Ancillary::default(), OTHER).unwrap();
    assert_eq!(recv(&s, 10).unwrap().0, b"ping");
    send(&s, b"pong").unwrap();
    assert_eq!(recv(&c, 10).unwrap().0, b"pong");
    // A datagram socket cannot connect to it.
    let d = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    assert_eq!(d.connect(&target, ME), Err(Errno::EPROTOTYPE));
    // Closing the listener resets the connection nobody accepted, and
    // frees the name.
    l.close();
    assert_eq!(recv(&c2, 10).err(), Some(Errno::ECONNRESET));
    assert!(e.ns.lookup_abstract(b"svc").is_none());
    let again = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    again.bind_abstract(b"svc").unwrap();
}

#[test]
fn path_names_live_in_the_file_system() {
    let e = env();
    let at = At::Cwd;
    let l = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    e.vfs.mkdir(&e.ctx, &e.root, at, b"/run", 0o755).unwrap();
    l.bind_path(&e.vfs, &e.ctx, &e.root, at, b"/run/sock").unwrap();
    let st = e.vfs.stat(&e.ctx, &e.root, at, b"/run/sock", false).unwrap();
    assert_eq!(st.kind(), Some(FileType::Socket));
    assert_eq!(st.perm(), 0o777 & !e.ctx.umask(), "0777 less the umask (unix(7))");
    let l2 = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(l2.bind_path(&e.vfs, &e.ctx, &e.root, at, b"/run/sock"), Err(Errno::EADDRINUSE));
    assert_eq!(l.local_name(), Address::Path(b"/run/sock".to_vec()));
    l.listen(5, ME).unwrap();
    // Lookups: missing, not a socket, permission, found.
    let lookup = |cred: &Cred, p: &[u8]| e.ns.lookup_path(&e.vfs, &e.ctx, cred, at, p).map(|_| ());
    assert_eq!(lookup(&e.root, b"/run/none"), Err(Errno::ENOENT));
    e.vfs.open(&e.ctx, &e.root, at, b"/run/file", OpenFlags::CREAT | OpenFlags::WRONLY, 0o666).unwrap();
    assert_eq!(lookup(&e.root, b"/run/file"), Err(Errno::ECONNREFUSED));
    let user = Cred::user(1000, 100);
    e.vfs.chmod(&e.ctx, &e.root, at, b"/run/sock", 0o777, true).unwrap();
    assert_eq!(lookup(&user, b"/run/sock"), Ok(()));
    e.vfs.chmod(&e.ctx, &e.root, at, b"/run/sock", 0o755, true).unwrap();
    assert_eq!(lookup(&user, b"/run/sock"), Err(Errno::EACCES), "connect needs write permission");
    let target = e.ns.lookup_path(&e.vfs, &e.ctx, &e.root, at, b"/run/sock").unwrap();
    let c = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    c.connect(&target, ME).unwrap();
    assert_eq!(c.peer_name(), Ok(Address::Path(b"/run/sock".to_vec())));
    // The name goes with the socket; the inode stays (unlink is the
    // owner's job, unix(7) NOTES), and connecting to it is refused.
    l.close();
    assert_eq!(lookup(&e.root, b"/run/sock"), Err(Errno::ECONNREFUSED));
    assert!(e.vfs.stat(&e.ctx, &e.root, at, b"/run/sock", false).is_ok());
    e.vfs.unlink(&e.ctx, &e.root, at, b"/run/sock").unwrap();
    let l3 = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    l3.bind_path(&e.vfs, &e.ctx, &e.root, at, b"/run/sock").unwrap();
}

#[test]
fn autobind_names_are_five_hex_digits() {
    let e = env();
    let s = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    s.autobind().unwrap();
    let Address::Abstract(name) = s.local_name() else { panic!("not abstract") };
    assert_eq!(name.len(), 5);
    assert!(name.iter().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    let t = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    t.autobind().unwrap();
    assert_ne!(t.local_name(), s.local_name());
    // SO_PASSCRED on an unbound socket that sends autobinds it.
    let r = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    r.bind_abstract(b"r").unwrap();
    let u = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    u.set_passcred(true);
    u.send(b"hi", Some(&r), Ancillary::default(), ME).unwrap();
    assert!(matches!(recv(&r, 10).unwrap().1.from, Address::Abstract(_)));
}

#[test]
fn datagrams_keep_boundaries_addresses_and_peers() {
    let e = env();
    let srv = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    srv.bind_abstract(b"dgram").unwrap();
    let cli = UnixSocket::new(&e.ns, SocketType::Datagram, OTHER);
    cli.bind_abstract(b"client").unwrap();
    assert_eq!(cli.send(b"x", None, Ancillary::default(), OTHER), Err(Errno::ENOTCONN));
    cli.send(b"first", Some(&srv), Ancillary::default(), OTHER).unwrap();
    cli.send(b"second!", Some(&srv), Ancillary::default(), OTHER).unwrap();
    assert_eq!(srv.bytes_readable(), Ok(5), "the next datagram's size");
    // Truncation: the rest of the datagram is gone.
    let (got, r) = recv(&srv, 3).unwrap();
    assert_eq!((got.as_slice(), r.full_len), (b"fir".as_slice(), 5));
    assert_eq!(r.from, Address::Abstract(b"client".to_vec()));
    let mut peek = [0u8; 16];
    assert_eq!(srv.recv(&mut peek, true).unwrap().len, 7);
    assert_eq!(recv(&srv, 16).unwrap().0, b"second!");
    // A connected datagram socket: default destination and only source.
    srv.connect(&cli, ME).unwrap();
    assert_eq!(srv.peer_name(), Ok(Address::Abstract(b"client".to_vec())));
    send(&srv, b"to client").unwrap();
    assert_eq!(recv(&cli, 32).unwrap().0, b"to client");
    assert_eq!(srv.send(b"x", Some(&cli), Ancillary::default(), ME), Err(Errno::EISCONN));
    let third = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    assert_eq!(third.send(b"x", Some(&srv), Ancillary::default(), ME), Err(Errno::EPERM));
    // Empty datagrams are datagrams.
    cli.send(b"", Some(&srv), Ancillary::default(), OTHER).unwrap();
    assert_eq!(recv(&srv, 4).unwrap().1.full_len, 0);
    // The peer is gone: refused.
    cli.close();
    assert_eq!(send(&srv, b"x"), Err(Errno::ECONNREFUSED));
    srv.disconnect().unwrap();
    assert_eq!(send(&srv, b"x"), Err(Errno::ENOTCONN));
    // A stream socket is the wrong type for a datagram.
    let st = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(srv.send(b"x", Some(&st), Ancillary::default(), ME), Err(Errno::EPROTOTYPE));
}

#[test]
fn a_kept_error_comes_once_before_what_is_queued() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Datagram, ME);
    assert_eq!(send(&a, b"333"), Ok(3));
    b.keep_error(Errno::EFAULT);
    assert!(b.poll().intersects(PollEvents::READABLE));
    assert_eq!(recv(&b, 8).map(|r| r.0), Err(Errno::EFAULT));
    assert_eq!(recv(&b, 8).map(|r| r.0), Ok(b"333".to_vec()));
    assert_eq!(recv(&b, 8).map(|r| r.0), Err(Errno::EAGAIN));
}

#[test]
fn a_full_datagram_queue_blocks_senders_until_read() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Datagram, ME);
    let mut sent = 0;
    while send(&a, b"0123456789").is_ok() {
        sent += 1;
    }
    assert_eq!(sent, MAX_DGRAM_QUEUE);
    assert_eq!(send(&a, b"x"), Err(Errno::EAGAIN));
    assert!(!a.poll().intersects(PollEvents::WRITABLE));
    let w = Arc::new(Counter::default());
    let _reg = a.wait_queue().register(PollEvents::WRITABLE, w.clone());
    recv(&b, 100).unwrap();
    assert!(w.0.load(Ordering::SeqCst) >= 1);
    assert_eq!(send(&a, b"x"), Ok(1));
    // unix(7): at most the doubled SO_SNDBUF less 32 bytes.
    let big = vec![0u8; DEFAULT_BUFFER - 31];
    assert_eq!(send(&a, &big), Err(Errno::EMSGSIZE));
}

#[test]
fn seqpacket_keeps_boundaries_over_a_connection() {
    let e = env();
    let l = UnixSocket::new(&e.ns, SocketType::SeqPacket, ME);
    l.bind_abstract(b"seq").unwrap();
    l.listen(4, ME).unwrap();
    let c = UnixSocket::new(&e.ns, SocketType::SeqPacket, ME);
    c.connect(&l, ME).unwrap();
    let s = l.accept().unwrap();
    send(&c, b"one").unwrap();
    send(&c, b"three").unwrap();
    let (got, r) = recv(&s, 2).unwrap();
    assert_eq!((got.as_slice(), r.full_len), (b"on".as_slice(), 3));
    assert_eq!(recv(&s, 10).unwrap().0, b"three");
    c.close();
    assert_eq!(recv(&s, 10).unwrap().1.len, 0);
    assert_eq!(send(&s, b"x"), Err(Errno::EPIPE));
}

#[test]
fn errors_on_the_wrong_state() {
    let e = env();
    let s = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(recv(&s, 4).err(), Some(Errno::ENOTCONN));
    assert_eq!(send(&s, b"x"), Err(Errno::ENOTCONN));
    assert_eq!(s.peer_name(), Err(Errno::ENOTCONN));
    assert_eq!(s.listen(1, ME), Err(Errno::EINVAL), "unbound");
    s.autobind().unwrap();
    s.listen(1, ME).unwrap();
    assert_eq!(recv(&s, 4).err(), Some(Errno::EINVAL));
    assert_eq!(s.bytes_readable(), Err(Errno::EINVAL));
    let target = UnixSocket::new(&e.ns, SocketType::Stream, ME);
    assert_eq!(s.connect(&target, ME), Err(Errno::EINVAL));
    let d = UnixSocket::new(&e.ns, SocketType::Datagram, ME);
    assert_eq!(d.listen(1, ME), Err(Errno::EOPNOTSUPP));
    assert_eq!(d.accept().err(), Some(Errno::EOPNOTSUPP));
    let rights = (0..=SCM_MAX_FD).map(|_| file(&e, Arc::new(poly_vfs::eventfd::EventFd::new(0, false)))).collect();
    let (a, _b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    assert_eq!(a.send(b"x", None, Ancillary { rights, creds: None }, ME), Err(Errno::EINVAL));
}

#[test]
fn descriptors_in_a_closed_queue_are_released() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    let (r, w) = poly_vfs::pipe::Pipe::pair(false);
    let reader = file(&e, Arc::new(r));
    let writer = file(&e, Arc::new(w));
    a.send(b"w", None, Ancillary { rights: vec![writer], creds: None }, ME).unwrap();
    assert_eq!(e.ns.in_flight(), 1);
    // The pipe's only write end is in flight: no end of file yet.
    let mut buf = [0u8; 4];
    assert_eq!(reader.read(&mut buf), Err(Errno::EAGAIN));
    // The receiver closes without reading: the write end goes with it.
    b.close();
    assert_eq!(e.ns.in_flight(), 0);
    assert_eq!(reader.read(&mut buf), Ok(0), "the last write end is closed");
}

#[test]
fn unix_files_close_their_socket_when_dropped() {
    let e = env();
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    let fa = file(&e, Arc::new(UnixFile::new(Arc::clone(&a))));
    let fb = file(&e, Arc::new(UnixFile::new(Arc::clone(&b))));
    assert_eq!(fa.write(b"through the file"), Ok(16));
    let mut buf = [0u8; 32];
    assert_eq!(fb.read(&mut buf), Ok(16));
    assert_eq!(fb.poll(), PollEvents::WRITABLE);
    drop(fa);
    assert!(a.is_closed());
    assert_eq!(fb.read(&mut buf), Ok(0));
}

#[test]
fn cycles_of_sockets_in_flight_are_collected() {
    let e = env();
    // A socket sent over its own connection: it holds itself.
    let (a, b) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    let fa = file(&e, Arc::new(UnixFile::new(Arc::clone(&a))));
    let fb = file(&e, Arc::new(UnixFile::new(Arc::clone(&b))));
    b.send(b"x", None, Ancillary { rights: vec![fa.clone()], creds: None }, ME).unwrap();
    // Two sockets in each other's queues.
    let (c, d) = UnixSocket::pair(&e.ns, SocketType::Datagram, ME);
    let fc = file(&e, Arc::new(UnixFile::new(Arc::clone(&c))));
    let fd = file(&e, Arc::new(UnixFile::new(Arc::clone(&d))));
    c.send(b"d", None, Ancillary { rights: vec![fd.clone()], creds: None }, ME).unwrap();
    d.send(b"c", None, Ancillary { rights: vec![fc.clone()], creds: None }, ME).unwrap();
    // One that is still reachable: a pipe end in a queue of a socket a
    // process holds.
    let (keep, other) = UnixSocket::pair(&e.ns, SocketType::Stream, ME);
    let fkeep = file(&e, Arc::new(UnixFile::new(Arc::clone(&keep))));
    let (r, _w) = poly_vfs::pipe::Pipe::pair(false);
    other.send(b"p", None, Ancillary { rights: vec![file(&e, Arc::new(r))], creds: None }, ME).unwrap();
    assert_eq!(e.ns.in_flight(), 4);
    assert_eq!(e.ns.collect(), 0, "everything is still held by descriptors");
    // The processes close everything but `fkeep`.
    drop((fa, fb, fc, fd));
    assert!(!a.is_closed() && !c.is_closed() && !d.is_closed(), "the cycles hold themselves");
    assert!(b.is_closed());
    assert_eq!(e.ns.collect(), 3);
    assert!(a.is_closed() && c.is_closed() && d.is_closed());
    assert_eq!(e.ns.in_flight(), 1, "the pipe end stays in keep's queue");
    let (_, got) = recv(&keep, 4).unwrap();
    assert_eq!(got.rights.len(), 1);
    drop(fkeep);
    assert_eq!(e.ns.in_flight(), 0);
}
