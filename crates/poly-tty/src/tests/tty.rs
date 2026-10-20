//! Terminals against a fake host: the controlling terminal, job control,
//! signals from input, the ioctls, blocking reads and writes.

use super::fake::{FakeHost, Screen, Target, USER, act_as, caller};
use crate::device::{Console, ControllingTty};
use crate::ioctl::*;
use crate::sig::*;
use crate::termios::{Termios, Winsize, cc, lflag};
use crate::{Tty, TtyFile};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use poly_vfs::{CharDevice, Errno, OpenFlags, PollEvents};
use std::thread;
use std::time::{Duration, Instant};

/// A terminal on a recording driver.
fn terminal() -> (Arc<FakeHost>, Arc<Screen>, Arc<Tty>) {
    let host = FakeHost::new();
    let screen = Arc::new(Screen::default());
    let tty = Tty::new(&host.ttys(), "ttyT", screen.clone(), Termios::sane(38400), Winsize::new(24, 80));
    (host, screen, tty)
}

/// Session 10 (leader 10, groups 10 and 11) opens the terminal, which
/// becomes its controlling terminal with group 10 in the foreground.
fn session(host: &FakeHost, tty: &Arc<Tty>) -> Arc<TtyFile> {
    host.groups.lock().unwrap().extend([(10, 10), (11, 10), (20, 20)]);
    act_as(Some(caller(10, 10, 10)));
    let f = tty.open(OpenFlags::RDWR, true).unwrap();
    assert_eq!(tty.session(), (Some(10), Some(10)));
    f
}

fn read(f: &TtyFile, max: usize) -> Result<Vec<u8>, Errno> {
    let mut buf = vec![0u8; max];
    f.read(0, &mut buf, true).map(|n| buf[..n].to_vec())
}

#[test]
fn a_session_leader_acquires_its_controlling_terminal() {
    let (host, screen, tty) = terminal();
    // Not a leader, or O_NOCTTY: no.
    act_as(Some(caller(11, 10, 10)));
    let a = tty.open(OpenFlags::RDWR, true).unwrap();
    act_as(Some(caller(10, 10, 10)));
    let b = tty.open(OpenFlags::RDWR | OpenFlags::NOCTTY, true).unwrap();
    assert_eq!(tty.session(), (None, None));
    // Through /dev/console: no either.
    let console = Console::new(Some(tty.clone()));
    let c = console.open(OpenFlags::RDWR).unwrap().unwrap();
    assert_eq!(tty.session(), (None, None));
    let d = session(&host, &tty);
    assert!(tty.ttys().controlling().is_some_and(|t| Arc::ptr_eq(&t, &tty)));
    // Another session's leader cannot take it.
    act_as(Some(caller(20, 20, 20)));
    let _e = tty.open(OpenFlags::RDWR, true).unwrap();
    assert_eq!(tty.session(), (Some(10), Some(10)));
    assert_eq!(*screen.opened.lock().unwrap(), [true], "the driver hears of the first open");
    drop((a, b, c, d, _e));
    assert_eq!(*screen.opened.lock().unwrap(), [true, false], "and of the last close");
    assert_eq!(tty.opens(), 0);
}

#[test]
fn dev_tty_opens_the_controlling_terminal() {
    let (_host, screen, tty) = terminal();
    act_as(Some(caller(30, 30, 30)));
    let dev = ControllingTty::new(tty.ttys().clone());
    assert_eq!(dev.open(OpenFlags::RDWR).map(|_| ()), Err(Errno::ENXIO), "no controlling terminal");
    let _f = tty.open(OpenFlags::RDWR, true).unwrap();
    let g = dev.open(OpenFlags::RDWR).unwrap().unwrap();
    g.write(0, b"via /dev/tty", false).unwrap();
    assert_eq!(tty.opens(), 2);
    assert_eq!(screen.text(), "via /dev/tty");
}

#[test]
fn typed_signal_characters_reach_the_foreground_group() {
    let (host, screen, tty) = terminal();
    let f = session(&host, &tty);
    act_as(None);
    tty.receive(b"abc\x03");
    assert_eq!(host.take_signals(), [(Target::Group(10), SIGINT)]);
    assert_eq!(screen.text(), "abc^C");
    tty.receive(b"\x1a\x1c");
    assert_eq!(host.take_signals(), [(Target::Group(10), SIGTSTP), (Target::Group(10), SIGQUIT)]);
    // Without a session nobody is signalled.
    let (host2, _, other) = terminal();
    other.receive(b"\x03");
    assert!(host2.take_signals().is_empty());
    drop(f);
}

#[test]
fn background_reads_get_sigttin_or_eio() {
    let (host, _screen, tty) = terminal();
    let f = session(&host, &tty);
    tty.receive(b"line\n");
    // Group 11 of the session is in the background.
    act_as(Some(caller(12, 11, 10)));
    assert_eq!(read(&f, 10), Err(Errno::EINTR));
    assert_eq!(host.take_signals(), [(Target::Group(11), SIGTTIN)]);
    host.orphaned.lock().unwrap().insert(11);
    assert_eq!(read(&f, 10), Err(Errno::EIO), "orphaned group");
    host.orphaned.lock().unwrap().clear();
    host.ignored.lock().unwrap().insert(SIGTTIN);
    assert_eq!(read(&f, 10), Err(Errno::EIO), "SIGTTIN ignored or blocked");
    assert!(host.take_signals().is_empty());
    // Another session's process is not subject to it; the foreground is.
    act_as(Some(caller(20, 20, 20)));
    assert_eq!(read(&f, 2).unwrap(), b"li");
    act_as(Some(caller(10, 10, 10)));
    assert_eq!(read(&f, 10).unwrap(), b"ne\n");
}

#[test]
fn background_writes_with_tostop_get_sigttou() {
    let (host, screen, tty) = terminal();
    let f = session(&host, &tty);
    act_as(Some(caller(12, 11, 10)));
    assert_eq!(f.write(0, b"bg\n", false), Ok(3), "without TOSTOP background writes go out");
    assert_eq!(screen.text(), "bg\r\n");
    let mut t = tty.termios();
    t.lflag |= lflag::TOSTOP;
    host.poke(USER, &t.to_bytes());
    // Changing the terminal from the background: SIGTTOU whatever TOSTOP.
    assert_eq!(f.ioctl(TCSETS, USER), Err(Errno::EINTR));
    assert_eq!(host.take_signals(), [(Target::Group(11), SIGTTOU)]);
    act_as(Some(caller(10, 10, 10)));
    assert_eq!(f.ioctl(TCSETS, USER), Ok(0));
    act_as(Some(caller(12, 11, 10)));
    assert_eq!(f.write(0, b"x", false), Err(Errno::EINTR));
    assert_eq!(host.take_signals(), [(Target::Group(11), SIGTTOU)]);
    host.orphaned.lock().unwrap().insert(11);
    assert_eq!(f.write(0, b"x", false), Err(Errno::EIO));
    host.ignored.lock().unwrap().insert(SIGTTOU);
    assert_eq!(f.write(0, b"ok", false), Ok(2), "blocking SIGTTOU lets the write through");
    assert!(host.take_signals().is_empty());
    assert_eq!(screen.text(), "ok");
}

#[test]
fn termios_and_window_size_ioctls() {
    let (host, screen, tty) = terminal();
    let f = session(&host, &tty);
    assert_eq!(f.ioctl(TCGETS, USER), Ok(0));
    let got = Termios::sane(9600).from_bytes(&host.peek(USER, Termios::SIZE).try_into().unwrap());
    assert_eq!(got, tty.termios());
    // Raw mode, no echo: one byte at a time.
    let mut raw = got;
    raw.lflag &= !(lflag::ICANON | lflag::ECHO);
    raw.cc[cc::VMIN] = 1;
    host.poke(USER, &raw.to_bytes());
    tty.receive(b"x");
    assert_eq!(f.ioctl(TCSETSF, USER), Ok(0));
    assert_eq!(tty.termios(), raw);
    assert_eq!(read(&f, 10), Err(Errno::EAGAIN), "TCSETSF flushed the input");
    assert_eq!(screen.text(), "x", "echoed before");
    tty.receive(b"yz");
    assert!(screen.text().is_empty(), "no echo");
    assert_eq!(read(&f, 10).unwrap(), b"yz");
    // termios2 and termio views of the same attributes.
    assert_eq!(f.ioctl(TCGETS2, USER), Ok(0));
    assert_eq!(&host.peek(USER, Termios::SIZE2)[..], &raw.to_bytes2()[..]);
    assert_eq!(f.ioctl(TCGETA, USER), Ok(0));
    assert_eq!(&host.peek(USER, Termios::SIZE_TERMIO)[..], &raw.to_termio()[..]);
    // Window size: a change sends SIGWINCH to the foreground group.
    assert_eq!(f.ioctl(TIOCGWINSZ, USER), Ok(0));
    assert_eq!(Winsize::from_bytes(&host.peek(USER, 8).try_into().unwrap()), Winsize::new(24, 80));
    host.poke(USER, &Winsize::new(50, 132).to_bytes());
    assert_eq!(f.ioctl(TIOCSWINSZ, USER), Ok(0));
    assert_eq!(host.take_signals(), [(Target::Group(10), SIGWINCH)]);
    assert_eq!(f.ioctl(TIOCSWINSZ, USER), Ok(0));
    assert!(host.take_signals().is_empty(), "no change, no signal");
    assert_eq!(tty.winsize(), Winsize::new(50, 132));
    // Bad pointers.
    assert_eq!(f.ioctl(TCGETS, 0), Err(Errno::EFAULT));
    assert_eq!(f.ioctl(0x1234, USER), Err(Errno::ENOTTY));
}

#[test]
fn process_groups_and_sessions() {
    let (host, _screen, tty) = terminal();
    let f = session(&host, &tty);
    assert_eq!(f.ioctl(TIOCGPGRP, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 10);
    assert_eq!(f.ioctl(TIOCGSID, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 10);
    // A group of another session, a missing one, a negative one.
    host.poke(USER, &20i32.to_le_bytes());
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Err(Errno::EPERM));
    host.poke(USER, &99i32.to_le_bytes());
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Err(Errno::EPERM));
    host.poke(USER, &(-1i32).to_le_bytes());
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Err(Errno::EINVAL));
    host.poke(USER, &11i32.to_le_bytes());
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Ok(0));
    assert_eq!(tty.session(), (Some(10), Some(11)));
    // Now 10 is in the background: tcsetpgrp from there gets SIGTTOU.
    host.poke(USER, &10i32.to_le_bytes());
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Err(Errno::EINTR));
    assert_eq!(host.take_signals(), [(Target::Group(10), SIGTTOU)]);
    // Not the caller's controlling terminal: ENOTTY.
    act_as(Some(caller(20, 20, 20)));
    assert_eq!(f.ioctl(TIOCGPGRP, USER), Err(Errno::ENOTTY));
    assert_eq!(f.ioctl(TIOCGSID, USER), Err(Errno::ENOTTY));
    assert_eq!(f.ioctl(TIOCSPGRP, USER), Err(Errno::ENOTTY));
}

#[test]
fn tiocsctty_and_tiocnotty() {
    let (host, _screen, tty) = terminal();
    host.groups.lock().unwrap().extend([(10, 10), (11, 10), (20, 20)]);
    act_as(Some(caller(10, 10, 10)));
    let f = tty.open(OpenFlags::RDWR | OpenFlags::NOCTTY, true).unwrap();
    act_as(Some(caller(11, 11, 10)));
    assert_eq!(f.ioctl(TIOCSCTTY, 0), Err(Errno::EPERM), "not a session leader");
    act_as(Some(caller(10, 10, 10)));
    assert_eq!(f.ioctl(TIOCSCTTY, 0), Ok(0));
    assert_eq!(f.ioctl(TIOCSCTTY, 0), Ok(0), "already its terminal");
    assert_eq!(tty.session(), (Some(10), Some(10)));
    // Stealing it needs privilege and arg 1.
    act_as(Some(caller(20, 20, 20)));
    assert_eq!(f.ioctl(TIOCSCTTY, 1), Err(Errno::EPERM));
    let mut root = caller(20, 20, 20);
    root.privileged = true;
    act_as(Some(root));
    assert_eq!(f.ioctl(TIOCSCTTY, 0), Err(Errno::EPERM));
    assert_eq!(f.ioctl(TIOCSCTTY, 1), Ok(0));
    assert_eq!(tty.session(), (Some(20), Some(20)));
    // A non-leader gives it up for itself only.
    act_as(Some(caller(21, 20, 20)));
    assert_eq!(f.ioctl(TIOCNOTTY, 0), Ok(0));
    assert_eq!(f.ioctl(TIOCGPGRP, USER), Err(Errno::ENOTTY));
    assert_eq!(tty.session(), (Some(20), Some(20)));
    // The leader gives it up for the session: SIGHUP and SIGCONT to the
    // foreground group.
    act_as(Some(root));
    assert_eq!(f.ioctl(TIOCNOTTY, 0), Ok(0));
    assert_eq!(host.take_signals(), [(Target::Group(20), SIGHUP), (Target::Group(20), SIGCONT)]);
    assert_eq!(tty.session(), (None, None));
    assert_eq!(f.ioctl(TIOCNOTTY, 0), Err(Errno::ENOTTY));
}

#[test]
fn the_controlling_process_ending_hangs_up_its_session() {
    let (host, _screen, tty) = terminal();
    let _f = session(&host, &tty);
    let ttys = tty.ttys().clone();
    ttys.process_exited(12, 10);
    assert_eq!(tty.session(), (Some(10), Some(10)), "not the leader");
    ttys.process_exited(10, 10);
    assert_eq!(host.take_signals(), [(Target::Group(10), SIGHUP)]);
    assert_eq!(tty.session(), (None, None));
    // Free for another session.
    act_as(Some(caller(20, 20, 20)));
    let _g = tty.open(OpenFlags::RDWR, true).unwrap();
    assert_eq!(tty.session(), (Some(20), Some(20)));
}

#[test]
fn queues_flow_and_injected_input() {
    let (host, screen, tty) = terminal();
    let f = session(&host, &tty);
    tty.receive(b"ab\ncd\n");
    assert_eq!(f.ioctl(FIONREAD, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 6);
    assert_eq!(f.ioctl(TIOCOUTQ, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 0);
    assert_eq!(f.ioctl(TCFLSH, 0), Ok(0));
    assert_eq!(read(&f, 10), Err(Errno::EAGAIN));
    assert_eq!(f.ioctl(TCFLSH, 1), Ok(0));
    assert_eq!(screen.flushed.load(core::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(f.ioctl(TCFLSH, 7), Err(Errno::EINVAL));
    // tcflow: output off, on; send STOP / START.
    screen.take();
    assert_eq!(f.ioctl(TCXONC, 0), Ok(0));
    assert_eq!(f.write(0, b"held", true), Err(Errno::EAGAIN));
    assert!(!tty.poll().contains(PollEvents::OUT));
    assert_eq!(f.ioctl(TCXONC, 1), Ok(0));
    assert_eq!(f.write(0, b"out", true), Ok(3));
    assert_eq!(f.ioctl(TCXONC, 2), Ok(0));
    assert_eq!(f.ioctl(TCXONC, 3), Ok(0));
    assert_eq!(screen.take(), b"out\x13\x11");
    assert_eq!(f.ioctl(TCXONC, 9), Err(Errno::EINVAL));
    assert_eq!(f.ioctl(TCSBRK, 1), Ok(0));
    // TIOCSTI: on the caller's own terminal (or with privilege).
    host.poke(USER, b"Z");
    assert_eq!(f.ioctl(TIOCSTI, USER), Ok(0));
    tty.receive(b"\n");
    assert_eq!(read(&f, 10).unwrap(), b"Z\n");
    act_as(Some(caller(20, 20, 20)));
    assert_eq!(f.ioctl(TIOCSTI, USER), Err(Errno::EPERM));
    // Exclusive mode: further opens fail unless privileged.
    act_as(Some(caller(10, 10, 10)));
    assert_eq!(f.ioctl(TIOCEXCL, 0), Ok(0));
    assert_eq!(f.ioctl(TIOCGEXCL, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 1);
    assert_eq!(tty.open(OpenFlags::RDWR, true).map(|_| ()), Err(Errno::EBUSY));
    assert_eq!(f.ioctl(TIOCNXCL, 0), Ok(0));
    assert!(tty.open(OpenFlags::RDWR, true).is_ok());
    // The line discipline number.
    assert_eq!(f.ioctl(TIOCGETD, USER), Ok(0));
    assert_eq!(host.peek_int(USER), 0);
    host.poke(USER, &1i32.to_le_bytes());
    assert_eq!(f.ioctl(TIOCSETD, USER), Err(Errno::EINVAL));
}

#[test]
fn blocking_reads_wake_on_input_and_on_signals() {
    let (host, _screen, tty) = terminal();
    let f = session(&host, &tty);
    let reader = {
        let f = f.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            let mut buf = [0u8; 16];
            f.read(0, &mut buf, false).map(|n| buf[..n].to_vec())
        })
    };
    host.wait_blocked(1);
    assert!(!tty.queue().is_empty(), "the reader waits on the terminal's queue");
    act_as(None);
    tty.receive(b"partial");
    thread::sleep(Duration::from_millis(20));
    assert!(!reader.is_finished(), "no line yet");
    tty.receive(b" line\r");
    assert_eq!(reader.join().unwrap().unwrap(), b"partial line\n");

    let reader = {
        let f = f.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            f.read(0, &mut [0u8; 4], false)
        })
    };
    host.wait_blocked(1);
    host.interrupt();
    assert_eq!(reader.join().unwrap(), Err(Errno::EINTR));
    assert!(tty.poll().contains(PollEvents::OUT) && !tty.poll().contains(PollEvents::IN));
    tty.receive(b"x\n");
    assert!(tty.poll().contains(PollEvents::IN));
}

#[test]
fn time_ends_a_non_canonical_read() {
    let (host, _screen, tty) = terminal();
    let f = session(&host, &tty);
    let mut t = tty.termios();
    t.lflag &= !lflag::ICANON;
    t.cc[cc::VMIN] = 0;
    t.cc[cc::VTIME] = 1;
    tty.set_termios(t, false);
    let start = Instant::now();
    assert_eq!(f.read(0, &mut [0u8; 4], false), Ok(0));
    assert!(start.elapsed() >= Duration::from_millis(100), "TIME is in tenths of a second");
    tty.receive(b"q");
    assert_eq!(f.read(0, &mut [0u8; 4], false), Ok(1));
}

#[test]
fn stopped_output_holds_writers_until_start() {
    let (host, screen, tty) = terminal();
    let f = session(&host, &tty);
    tty.receive(b"\x13");
    let writer = {
        let f = f.clone();
        thread::spawn(move || {
            act_as(Some(caller(10, 10, 10)));
            f.write(0, b"later\n", false)
        })
    };
    host.wait_blocked(1);
    assert!(screen.take().is_empty());
    act_as(None);
    tty.receive(b"\x11");
    assert_eq!(writer.join().unwrap(), Ok(6));
    assert_eq!(screen.take(), b"later\r\n");
}
