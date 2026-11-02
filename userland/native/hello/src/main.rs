//! `/bin/native-hello`: the first native Poly program (ADR 0125), the
//! native boot test's first process. It prints, reads its arguments and
//! environment, allocates, keeps time, writes and reads a file, spawns
//! native children (one exits with a status, one panics, one faults) and,
//! when it is there, the Linux hello world `/bin/hello` on the same
//! console handle, and checks each. It ends with
//! `native-hello: done, <n> failed` and exit status 0 if nothing failed.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use poly_rt::abi::ExitStatus;
use poly_rt::io::{Read, Write};
use poly_rt::process::{Child, Command};
use poly_rt::time::Instant;
use poly_rt::{Error, env, fs, mem, net, poll, println, process, time};

poly_rt::entry!(main);

/// Count a failed check and say which.
fn check(failed: &mut u32, what: &str, ok: bool) {
    if !ok {
        println!("native-hello: FAIL {what}");
        *failed += 1;
    }
}

fn main() -> i32 {
    let mut failed = 0;
    println!("native-hello: hello from Poly's native user space (pid {})", process::id());

    let args: Vec<&str> = env::args_str().collect();
    println!("native-hello: arguments {args:?}");
    check(&mut failed, "argv[0] names the program", args.first().is_some_and(|a| a.ends_with("native-hello")));

    // The heap: many small objects (slabs), one large run, reuse.
    let numbers: Vec<u64> = (0..100_000).collect();
    check(&mut failed, "a vector of 100000 numbers", numbers.iter().sum::<u64>() == 4_999_950_000);
    let big = alloc::vec![0xa5u8; 3 << 20];
    check(&mut failed, "a 3 MiB allocation", big.len() == 3 << 20 && big.iter().all(|&b| b == 0xa5));
    drop(big);
    let mut map = BTreeMap::new();
    for i in 0..2000u32 {
        map.insert(format!("key-{i:04}"), String::from("value").repeat(1 + i as usize % 7));
    }
    for round in 0..20 {
        let strings: Vec<String> = (0..500).map(|i| format!("{round}:{i}")).collect();
        check(&mut failed, "freed memory is reused", strings.len() == 500);
    }
    check(&mut failed, "a map of 2000 strings", map.len() == 2000 && map["key-1999"].len() == 5 * 5);
    println!("native-hello: heap ok ({} map entries)", map.len());

    // Time.
    let start = Instant::now();
    check(&mut failed, "sleep", time::sleep(Duration::from_millis(20)).is_ok());
    let slept = start.elapsed();
    check(&mut failed, "the monotonic clock moved by the sleep", slept >= Duration::from_millis(20));
    check(&mut failed, "the wall clock is past 2023", time::since_epoch().as_secs() > 1_700_000_000);
    println!("native-hello: slept {} ms", slept.as_millis());

    // Files, through the same VFS Linux programs use.
    let text = b"written by a native program\n";
    check(&mut failed, "writing a file", fs::write("/native-hello.txt", text).is_ok());
    check(&mut failed, "reading it back", fs::read("/native-hello.txt").as_deref() == Ok(&text[..]));
    check(&mut failed, "a missing file", fs::read("/no/such/file").err() == Some(Error::NotFound));
    let motd = fs::read("/etc/hostname").unwrap_or_default();
    println!("native-hello: /etc/hostname is {:?}", core::str::from_utf8(&motd).unwrap_or("?").trim_end());

    // Children: a native one's exit status, a panic, a fault.
    let status = Command::new("/bin/native-child").args(["exit", "42"]).status();
    println!("native-hello: native-child exit 42 ended: {status:?}");
    check(&mut failed, "a native child's exit status", status == Ok(ExitStatus::Exited(42)));
    let status = Command::new("/bin/native-child").arg("panic").status();
    check(&mut failed, "a native child's panic exits 101", status == Ok(ExitStatus::Exited(101)));
    let status = Command::new("/bin/native-child").arg("fault").status();
    println!("native-hello: native-child fault ended: {status:?}");
    check(&mut failed, "a native child's page fault", status == Ok(ExitStatus::Faulted(14)));
    let missing = Command::new("/bin/no-such-program").status();
    check(&mut failed, "spawning a missing program", missing == Err(Error::NotFound));

    // A Linux program started by a native one, sharing its console handle.
    if fs::File::open("/bin/hello").is_ok() {
        let status = Command::new("/bin/hello").status();
        println!("native-hello: the Linux /bin/hello ended: {status:?}");
        check(&mut failed, "a Linux child", status == Ok(ExitStatus::Exited(0)));
    }

    abi_v2(&mut failed);

    println!("native-hello: done, {failed} failed");
    if failed == 0 { 0 } else { 1 }
}

/// The calls of ABI version 2: files and directories, pipes, memory
/// objects mapped twice, sockets passing handles to a child, poll, process
/// handles, ending a child.
fn abi_v2(failed: &mut u32) {
    // Directories, renames, metadata, named pipes.
    let ok = fs::create_dir_all("/tmp/native/a/b").is_ok()
        && fs::write("/tmp/native/a/one", b"1").is_ok()
        && fs::rename("/tmp/native/a/one", "/tmp/native/a/two").is_ok()
        && fs::make_fifo("/tmp/native/a/fifo", 0o600).is_ok();
    check(failed, "directories, a rename and a named pipe", ok);
    let mut names: Vec<String> =
        fs::read_dir("/tmp/native/a").unwrap_or_default().into_iter().map(|e| e.name).collect();
    names.sort();
    check(failed, "reading a directory", names == ["b", "fifo", "two"]);
    let meta = fs::metadata("/tmp/native/a/two");
    check(failed, "metadata of a file", meta.is_ok_and(|m| m.is_file() && m.len() == 1));
    let console = fs::metadata("/dev/console");
    check(failed, "a device's number", console.is_ok_and(|m| m.device() == 5 << 32 | 1));
    check(failed, "removing a tree", fs::remove_dir_all("/tmp/native").is_ok() && !fs::exists("/tmp/native"));

    // A pipe, read through poll.
    let Ok([r, w]) = poly_rt::sys::pipe() else {
        check(failed, "a pipe", false);
        return;
    };
    let (mut r, w) = (fs::File::from_handle(r), fs::File::from_handle(w));
    let mut e = [poll::entry(r.handle(), poll::READABLE)];
    check(failed, "an empty pipe is not readable", poll::poll(&mut e, Some(Duration::ZERO)) == Ok(0));
    check(failed, "writing a pipe", w.write_all(b"through the pipe").is_ok());
    check(failed, "a full pipe is readable", poll::poll(&mut e, None) == Ok(1) && e[0].revents & poll::READABLE != 0);
    let mut buf = [0u8; 32];
    check(failed, "reading a pipe", r.read(&mut buf).ok() == Some(16) && &buf[..16] == b"through the pipe");
    drop(w);
    let mut e = [poll::entry(r.handle(), poll::READABLE)];
    let _ = poll::poll(&mut e, None);
    check(failed, "a pipe without writers hangs up", e[0].revents & poll::HANGUP != 0);

    // A memory object, sealed and mapped twice: both mappings see the same
    // pages.
    let object = mem::memory_object(8192);
    let mapped = object.as_ref().ok().and_then(|o| {
        let a = mem::map_handle(o.handle(), 0, 8192, mem::READ | mem::WRITE, true).ok()?;
        let b = mem::map_handle(o.handle(), 4096, 4096, mem::READ, true).ok()?;
        Some((a, b))
    });
    match (&object, mapped) {
        (Ok(o), Some((a, b))) => {
            // SAFETY: `a` maps 8192 writable bytes of the object, `b` its
            // second page; both stay mapped for this block.
            let same = unsafe {
                a.add(4096 + 7).write_volatile(0x5a);
                b.add(7).read_volatile() == 0x5a
            };
            check(failed, "two mappings of a memory object share its pages", same);
            let sealed = mem::seal_object(o, mem::seal::SHRINK | mem::seal::GROW);
            check(failed, "sealing a memory object", sealed.is_ok() && o.set_len(100).is_err());
        }
        _ => check(failed, "a memory object mapped twice", false),
    }

    // Sockets: a handle (the read end of a pipe) passed to a native child,
    // which reads from it and answers on the socket.
    let pair = net::UnixStream::pair();
    let pipe = poly_rt::sys::pipe();
    match (pair, pipe) {
        (Ok((mut ours, theirs)), Ok([pr, pw])) => {
            let (pr, pw) = (fs::File::from_handle(pr), fs::File::from_handle(pw));
            let number = alloc::format!("{}", theirs.handle());
            let child = Command::new("/bin/native-child").args(["echo-handle", &number]).keep(theirs.handle()).spawn();
            // The child's end is the child's alone: its exit ends the stream.
            drop(theirs);
            let _ = pw.write_all(b"sent with a handle");
            drop(pw);
            let sent = ours.send_with_handles(b"x", &[pr.handle()]);
            check(failed, "sending a handle", sent == Ok(1));
            drop(pr);
            let mut answer = Vec::new();
            let _ = ours.read_to_end(&mut answer);
            check(failed, "a child read from a passed handle", answer == b"got: sent with a handle");
            let status = child.and_then(Child::wait);
            check(failed, "the socket child's status", status == Ok(ExitStatus::Exited(0)));
        }
        _ => check(failed, "a socket pair and a pipe", false),
    }

    // A listener with a name in the abstract name space and one at a path.
    let listener = net::UnixListener::bind_abstract(b"poly-native-test");
    let client = net::UnixStream::connect("/no/such/socket");
    check(failed, "connecting to nothing", client.err() == Some(Error::NotFound));
    let named = net::UnixListener::bind("/tmp/native-socket");
    let connected = named.as_ref().ok().and_then(|l| {
        let c = net::UnixStream::connect("/tmp/native-socket").ok()?;
        let s = l.accept().ok()?;
        Some((c, s))
    });
    match connected {
        Some((mut c, s)) => {
            let _ = c.write_all(b"ping");
            let mut b = [0u8; 4];
            let mut s = s;
            check(failed, "a connection by path", s.read_exact(&mut b).is_ok() && &b == b"ping");
            check(failed, "the peer's pid", s.peer().is_ok_and(|p| p.pid == process::id()));
        }
        None => check(failed, "a connection by path", false),
    }
    check(failed, "an abstract listener", listener.is_ok());
    let _ = fs::remove_file("/tmp/native-socket");

    // A process handle turns readable when the child ends; a child that
    // waits forever ends when told to.
    match Command::new("/bin/native-child").arg("wait").spawn() {
        Ok(child) => {
            check(failed, "a child that waits runs", child.try_wait() == Ok(None));
            let h = child.process_handle();
            let mut e = [poll::entry(h.as_ref().map_or(poll::NO_HANDLE, |f| f.handle()), poll::READABLE)];
            check(failed, "its handle is not readable yet", poll::poll(&mut e, Some(Duration::ZERO)) == Ok(0));
            check(failed, "terminating it", child.terminate().is_ok());
            let woke = poll::poll(&mut e, Some(Duration::from_secs(10)));
            check(failed, "its handle is readable once it ended", woke == Ok(1));
            let status = child.try_wait();
            println!("native-hello: the terminated child ended: {status:?}");
            check(failed, "it ended terminated", status == Ok(Some(ExitStatus::Signaled(15))));
        }
        Err(_) => check(failed, "a child that waits", false),
    }
    check(failed, "pid 1 is alive", process::alive(1));
    check(failed, "a pid nobody has", !process::alive(99_999));
    let ifs = poly_rt::system::interfaces();
    check(failed, "the loopback interface", ifs.iter().any(|i| i.name == "lo" && i.loopback && i.up));
    let (os, release, ..) = poly_rt::system::uname();
    println!("native-hello: {os} {release}, {} interface(s)", ifs.len());
    check(failed, "uname", !os.is_empty());
}
