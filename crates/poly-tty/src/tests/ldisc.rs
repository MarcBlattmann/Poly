//! The line discipline on its own: canonical editing, echo, signals,
//! flow control, the input mappings, MIN / TIME and output processing.

use crate::ldisc::{Effects, INPUT_BUFFER, LineDiscipline, MAX_LINE, Read};
use crate::sig::{SIGINT, SIGQUIT, SIGTSTP};
use crate::termios::{Termios, cc, ctrl, iflag, lflag, oflag};
use alloc::vec;
use alloc::vec::Vec;

const MS: u64 = 1_000_000;

fn sane() -> Termios {
    Termios::sane(38400)
}

fn with(f: impl FnOnce(&mut Termios)) -> Termios {
    let mut t = sane();
    f(&mut t);
    t
}

fn raw(min: u8, time: u8) -> Termios {
    with(|t| {
        t.lflag &= !(lflag::ICANON | lflag::ECHO | lflag::ISIG | lflag::IEXTEN);
        t.iflag &= !(iflag::ICRNL | iflag::IXON);
        t.cc[cc::VMIN] = min;
        t.cc[cc::VTIME] = time;
    })
}

/// Type `input` at time 0 and return the effects.
fn typed(l: &mut LineDiscipline, input: &[u8]) -> Effects {
    let mut fx = Effects::default();
    l.receive(input, 0, &mut fx);
    fx
}

/// What a read of up to `max` bytes returns now (time 0).
fn read(l: &mut LineDiscipline, max: usize) -> Result<Vec<u8>, Read> {
    let mut buf = vec![0u8; max];
    match l.read(&mut buf, 0, 0, &mut Effects::default()) {
        Read::Data(n) => Ok(buf[..n].to_vec()),
        other => Err(other),
    }
}

#[test]
fn canonical_lines_are_echoed_and_read_whole() {
    let mut l = LineDiscipline::new(sane());
    let fx = typed(&mut l, b"hel");
    assert_eq!(fx.output, b"hel");
    assert!(!fx.readable && !l.readable());
    assert_eq!(read(&mut l, 100), Err(Read::Wait { until: None }), "no line yet");
    // CR becomes NL (ICRNL), echoed as CR LF (ONLCR).
    let fx = typed(&mut l, b"lo\r");
    assert_eq!(fx.output, b"lo\r\n");
    assert!(fx.readable && l.readable());
    assert_eq!(l.readable_bytes(), 6);
    assert_eq!(read(&mut l, 100).unwrap(), b"hello\n");
    // Two lines: one read returns one line; a short buffer splits it.
    typed(&mut l, b"one\ntwo\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"one\n");
    assert_eq!(read(&mut l, 2).unwrap(), b"tw");
    assert_eq!(read(&mut l, 100).unwrap(), b"o\n");
}

#[test]
fn erase_kill_and_word_erase_edit_the_line() {
    let mut l = LineDiscipline::new(sane());
    let fx = typed(&mut l, b"abx\x7f");
    assert_eq!(fx.output, b"abx\x08 \x08", "ECHOE erases on the screen");
    typed(&mut l, b"\x7f\x7f\x7f\x7fz\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"z\n", "erase stops at the start of the line");

    // A control character shows as ^A: two columns to erase.
    let fx = typed(&mut l, b"a\x01\x7f");
    assert_eq!(fx.output, b"a^A\x08 \x08\x08 \x08");
    // A tab: back to the column it started at.
    let fx = typed(&mut l, b"\x7fab\t\x7f");
    assert_eq!(&fx.output[fx.output.len() - 6 * 3..], b"\x08 \x08".repeat(6).as_slice());
    typed(&mut l, b"\x15");
    // Word erase: blanks, then the word.
    typed(&mut l, b"one two  \x17x\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"one x\n");

    // KILL with ECHOKE erases each character; with ECHOK only, ^U and a
    // new line.
    let fx = typed(&mut l, b"abc\x15");
    assert_eq!(fx.output, [&b"abc"[..], &b"\x08 \x08".repeat(3)].concat());
    let mut k = LineDiscipline::new(with(|t| t.lflag &= !lflag::ECHOKE));
    let fx = typed(&mut k, b"abc\x15d\n");
    assert_eq!(fx.output, b"abc^U\r\nd\r\n");
    assert_eq!(read(&mut k, 100).unwrap(), b"d\n");
    // Without ECHOE the erase character itself is echoed.
    let mut e = LineDiscipline::new(with(|t| t.lflag &= !lflag::ECHOE));
    assert_eq!(typed(&mut e, b"ab\x7f").output, b"ab^?");
}

#[test]
fn utf8_erase_takes_whole_characters() {
    let mut l = LineDiscipline::new(with(|t| t.iflag |= iflag::IUTF8));
    let fx = typed(&mut l, "aé\x7f".as_bytes());
    assert!(fx.output.ends_with(b"\x08 \x08") && !fx.output.ends_with(b"\x08 \x08\x08 \x08"));
    typed(&mut l, b"\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"a\n");
}

#[test]
fn eof_eol_reprint_and_literal_next() {
    let mut l = LineDiscipline::new(with(|t| {
        t.cc[cc::VEOL] = b';';
        t.cc[cc::VEOL2] = b'|';
    }));
    // EOF sends the line without a delimiter; at the start of a line it
    // is end of file. It is not echoed.
    let fx = typed(&mut l, b"abc\x04");
    assert_eq!(fx.output, b"abc");
    assert_eq!(read(&mut l, 100).unwrap(), b"abc");
    typed(&mut l, b"\x04");
    assert_eq!(read(&mut l, 100), Err(Read::Eof));
    assert_eq!(read(&mut l, 100), Err(Read::Wait { until: None }));
    typed(&mut l, b"a;b|");
    assert_eq!(read(&mut l, 100).unwrap(), b"a;");
    assert_eq!(read(&mut l, 100).unwrap(), b"b|");
    // REPRINT shows the line again on a new one.
    let fx = typed(&mut l, b"xy\x12");
    assert_eq!(fx.output, b"xy^R\r\nxy");
    typed(&mut l, b"\x15");
    // LNEXT quotes the next byte: no signal, no line end.
    let fx = typed(&mut l, b"\x16\x03\x16\n!\n");
    assert!(fx.signals.is_empty());
    assert_eq!(read(&mut l, 100).unwrap(), b"\x03\n!\n");
    assert!(fx.output.starts_with(b"^\x08^C^\x08"));
}

#[test]
fn signal_characters_flush_and_signal() {
    let mut l = LineDiscipline::new(sane());
    typed(&mut l, b"pending\nhalf");
    let fx = typed(&mut l, b"\x03");
    assert_eq!(fx.signals, [SIGINT]);
    assert_eq!(fx.output, b"^C");
    assert!(fx.flush_output);
    assert_eq!(l.readable_bytes(), 0, "input flushed");
    assert_eq!(typed(&mut l, b"\x1c").signals, [SIGQUIT]);
    assert_eq!(typed(&mut l, b"\x1a").signals, [SIGTSTP]);
    // NOFLSH keeps the input; without ISIG they are data.
    let mut n = LineDiscipline::new(with(|t| t.lflag |= lflag::NOFLSH));
    typed(&mut n, b"keep\n");
    let fx = typed(&mut n, b"\x03");
    assert!(!fx.flush_output && fx.signals == [SIGINT]);
    assert_eq!(read(&mut n, 100).unwrap(), b"keep\n");
    let mut d = LineDiscipline::new(with(|t| t.lflag &= !lflag::ISIG));
    assert!(typed(&mut d, b"\x03\n").signals.is_empty());
    assert_eq!(read(&mut d, 100).unwrap(), b"\x03\n");
    // A disabled character never matches.
    let mut z = LineDiscipline::new(with(|t| t.cc[cc::VINTR] = 0));
    assert!(typed(&mut z, b"\0\x03").signals.is_empty());
}

#[test]
fn echo_flags() {
    // No ECHO: nothing; ECHONL: only new lines (canonical mode).
    let mut l = LineDiscipline::new(with(|t| t.lflag &= !lflag::ECHO));
    assert!(typed(&mut l, b"secret\n").output.is_empty());
    let mut l = LineDiscipline::new(with(|t| t.lflag = (t.lflag & !lflag::ECHO) | lflag::ECHONL));
    assert_eq!(typed(&mut l, b"secret\n").output, b"\r\n");
    // Without ECHOCTL control characters go out as they are.
    let mut l = LineDiscipline::new(with(|t| t.lflag &= !lflag::ECHOCTL));
    assert_eq!(typed(&mut l, b"\x01\t").output, b"\x01\t");
    // TAB is not shown as ^I with ECHOCTL either.
    let mut l = LineDiscipline::new(sane());
    assert_eq!(typed(&mut l, b"\t\x1b").output, b"\t^[");
}

#[test]
fn input_mappings() {
    let mut l = LineDiscipline::new(with(|t| t.iflag = (t.iflag & !iflag::ICRNL) | iflag::IGNCR | iflag::ISTRIP));
    typed(&mut l, b"a\rb\xe1\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"aba\n", "CR ignored, the eighth bit stripped");
    let mut l = LineDiscipline::new(with(|t| {
        t.iflag |= iflag::INLCR;
        t.lflag &= !lflag::ICANON;
    }));
    typed(&mut l, b"\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"\r");
    let mut l = LineDiscipline::new(with(|t| t.iflag |= iflag::IUCLC));
    typed(&mut l, b"ABc\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"abc\n");
}

#[test]
fn flow_control_stops_and_starts_output() {
    let mut l = LineDiscipline::new(sane());
    let fx = typed(&mut l, b"\x13");
    assert!(l.output_stopped() && fx.output.is_empty() && !fx.output_started);
    typed(&mut l, b"x");
    assert!(l.output_stopped(), "only START (or IXANY) restarts");
    let fx = typed(&mut l, b"\x11");
    assert!(!l.output_stopped() && fx.output_started);
    typed(&mut l, b"\n");
    assert_eq!(read(&mut l, 100).unwrap(), b"x\n", "START and STOP are not data");
    let mut a = LineDiscipline::new(with(|t| t.iflag |= iflag::IXANY));
    typed(&mut a, b"\x13");
    assert!(typed(&mut a, b"y").output_started && !a.output_stopped());
    // Without IXON they are data.
    let mut d = LineDiscipline::new(with(|t| t.iflag &= !iflag::IXON));
    typed(&mut d, b"\x13\x11\n");
    assert_eq!(read(&mut d, 100).unwrap(), b"\x13\x11\n");
    // tcflow
    assert!(!d.set_output_stopped(true));
    assert!(d.set_output_stopped(false));
}

#[test]
fn ixoff_throttles_the_sender() {
    let mut l = LineDiscipline::new(with(|t| {
        t.lflag &= !(lflag::ICANON | lflag::ECHO);
        t.iflag |= iflag::IXOFF;
    }));
    let fx = typed(&mut l, &[b'a'; INPUT_BUFFER - 100]);
    assert_eq!(fx.output, [ctrl(b'S')], "STOP once nearly full");
    let mut buf = vec![0u8; INPUT_BUFFER];
    let mut fx = Effects::default();
    assert_eq!(l.read(&mut buf, 0, 0, &mut fx), Read::Data(INPUT_BUFFER - 100));
    assert_eq!(fx.output, [ctrl(b'Q')], "START once drained");
}

#[test]
fn input_is_bounded() {
    // Canonical: 4095 characters, then only the delimiter.
    let mut l = LineDiscipline::new(with(|t| t.lflag &= !lflag::ECHO));
    typed(&mut l, &[b'x'; 5000]);
    assert_eq!(l.room(), usize::MAX, "a full line discards, so there is always room");
    typed(&mut l, b"\n");
    let line = read(&mut l, 10_000).unwrap();
    assert_eq!(line.len(), MAX_LINE + 1);
    assert_eq!(line.last(), Some(&b'\n'));
    // Raw: what does not fit is dropped (a bell with IMAXBEL and ECHO).
    let mut r = LineDiscipline::new(with(|t| t.lflag &= !lflag::ICANON));
    let fx = typed(&mut r, &[b'y'; INPUT_BUFFER + 3]);
    assert_eq!(r.readable_bytes(), INPUT_BUFFER);
    assert_eq!(r.room(), 0);
    assert!(fx.output.ends_with(b"\x07\x07\x07"));
}

#[test]
fn min_and_time() {
    // MIN 0, TIME 0: whatever is there, maybe nothing.
    let mut l = LineDiscipline::new(raw(0, 0));
    assert_eq!(read(&mut l, 10), Ok(vec![]));
    typed(&mut l, b"ab");
    assert_eq!(read(&mut l, 10).unwrap(), b"ab");
    // MIN 3: waits for three bytes (or fewer if fewer are asked for).
    let mut l = LineDiscipline::new(raw(3, 0));
    typed(&mut l, b"ab");
    assert_eq!(read(&mut l, 10), Err(Read::Wait { until: None }));
    assert_eq!(read(&mut l, 2).unwrap(), b"ab");
    typed(&mut l, b"cde");
    assert_eq!(read(&mut l, 10).unwrap(), b"cde");
    // MIN 0, TIME 5: until a byte comes or 500 ms after the read started.
    let mut l = LineDiscipline::new(raw(0, 5));
    let mut buf = [0u8; 4];
    let fx = &mut Effects::default();
    assert_eq!(l.read(&mut buf, 1000, 1000, fx), Read::Wait { until: Some(1000 + 500 * MS) });
    assert_eq!(l.read(&mut buf, 1000, 1000 + 500 * MS, fx), Read::Data(0));
    l.receive(b"q", 1000 + 10 * MS, fx);
    assert_eq!(l.read(&mut buf, 1000, 1000 + 10 * MS, fx), Read::Data(1));
    // MIN 2, TIME 1: no timer before the first byte; then 100 ms after
    // the latest one.
    let mut l = LineDiscipline::new(raw(2, 1));
    assert_eq!(l.read(&mut buf, 0, 0, fx), Read::Wait { until: None });
    l.receive(b"x", 50 * MS, fx);
    assert_eq!(l.read(&mut buf, 0, 60 * MS, fx), Read::Wait { until: Some(150 * MS) });
    assert_eq!(l.read(&mut buf, 0, 150 * MS, fx), Read::Data(1));
    l.receive(b"yz", 200 * MS, fx);
    assert_eq!(l.read(&mut buf, 0, 200 * MS, fx), Read::Data(2), "MIN reached");
}

#[test]
fn output_processing() {
    let mut l = LineDiscipline::new(sane());
    let mut out = Vec::new();
    assert_eq!(l.output(b"a\nb\r", usize::MAX, &mut out), 4);
    assert_eq!(out, b"a\r\nb\r");
    // Room for part of the expansion only: the byte is not taken.
    let mut out = Vec::new();
    assert_eq!(l.output(b"x\n", 2, &mut out), 1);
    assert_eq!(out, b"x");
    let mut l = LineDiscipline::new(with(|t| t.oflag |= oflag::OCRNL | oflag::ONOCR | oflag::TAB3 | oflag::OLCUC));
    let mut out = Vec::new();
    l.output(b"\rab\tc\r", usize::MAX, &mut out);
    assert_eq!(out, b"AB      C\n", "CR at column 0 dropped, tabs expanded, CR as NL");
    let mut l = LineDiscipline::new(with(|t| t.oflag &= !oflag::OPOST));
    let mut out = Vec::new();
    l.output(b"a\n\t", usize::MAX, &mut out);
    assert_eq!(out, b"a\n\t");
}

#[test]
fn switching_modes_keeps_input() {
    let mut l = LineDiscipline::new(sane());
    typed(&mut l, b"half");
    let mut fx = Effects::default();
    l.set_termios(raw(1, 0), false, &mut fx);
    assert!(fx.readable);
    assert_eq!(read(&mut l, 100).unwrap(), b"half");
    typed(&mut l, b"raw");
    l.set_termios(sane(), false, &mut fx);
    assert_eq!(read(&mut l, 100).unwrap(), b"raw", "unread input becomes a line");
    typed(&mut l, b"x\n");
    l.set_termios(sane(), true, &mut fx);
    assert_eq!(l.readable_bytes(), 0, "TCSETSF flushes");
    // Turning IXON off restarts stopped output.
    typed(&mut l, b"\x13");
    let mut fx = Effects::default();
    l.set_termios(with(|t| t.iflag &= !iflag::IXON), false, &mut fx);
    assert!(fx.output_started && !l.output_stopped());
}
