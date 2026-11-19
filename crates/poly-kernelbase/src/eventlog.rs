//! The event log of `advapi32` (ADR 0227): `RegisterEventSourceA/W`,
//! `ReportEventA/W`, `DeregisterEventSource`, `OpenEventLogA/W`,
//! `ReadEventLogW` and `CloseEventLog`, as the sheets `RegisterEventSourceA`,
//! `ReportEventA`, `EventLogSourceRegistration` and `RegisterEventSourceW`
//! observe them.
//!
//! Poly has no event log service. A log is a file of Poly's own that every
//! process appends to: the `EVENTLOGRECORD`s (`winnt.h`, Learn:
//! "EVENTLOGRECORD") one after the other, oldest first, numbered from 1.
//! Writing opens the file without sharing, so two writers never number two
//! records alike; reading reads it whole. Every source writes to the
//! Application log whether or not it is registered below
//! `HKLM\SYSTEM\CurrentControlSet\Services\EventLog` (`EventLogSourceRegistration.yaml`
//! `unregistered`, `registered`), and a log name that is no log reads the
//! Application log (`read_back`).
//!
//! This module has the decisions and the formats; the DLL keeps the handle
//! table ([`Handles`]) and does the file I/O.

use alloc::vec::Vec;

/// `ELF_LOG_SIGNATURE` (`winnt.h`): `EVENTLOGRECORD.Reserved`.
pub const ELF_LOG_SIGNATURE: u32 = 0x654C_664C;
/// The fixed part of `EVENTLOGRECORD` (`winnt.h`): the source name follows.
pub const RECORD_HEADER: usize = 56;

/// `ReadEventLog` flags (`winnt.h`).
pub const EVENTLOG_SEQUENTIAL_READ: u32 = 0x1;
pub const EVENTLOG_SEEK_READ: u32 = 0x2;
pub const EVENTLOG_FORWARDS_READ: u32 = 0x4;
pub const EVENTLOG_BACKWARDS_READ: u32 = 0x8;

/// The event types (`winnt.h`): `EVENTLOG_SUCCESS`, `EVENTLOG_ERROR_TYPE`,
/// `EVENTLOG_WARNING_TYPE`, `EVENTLOG_INFORMATION_TYPE`,
/// `EVENTLOG_AUDIT_SUCCESS`, `EVENTLOG_AUDIT_FAILURE`.
pub const EVENT_TYPES: [u16; 6] = [0x0, 0x1, 0x2, 0x4, 0x8, 0x10];

/// Win32 errors (`winerror.h`).
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_BAD_COMMAND: u32 = 22;
/// What `ReportEventA` returns for a type outside [`EVENT_TYPES`]
/// (`ReportEventA.yaml` `types`: 23, `ERROR_CRC`, the value Windows 11 gives).
pub const ERROR_BAD_EVENT_TYPE: u32 = 23;
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const RPC_S_SERVER_UNAVAILABLE: u32 = 1722;

/// The logs Poly keeps: a log name other than these reads and writes the
/// Application log.
pub const APPLICATION: &str = "Application";
pub const SYSTEM: &str = "System";

/// The extension of a log's file.
pub const LOG_EXTENSION: &str = ".evtlog";

/// One event to write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub source: Vec<u16>,
    pub computer: Vec<u16>,
    pub event_id: u32,
    pub kind: u16,
    pub category: u16,
    pub strings: Vec<Vec<u16>>,
    pub sid: Vec<u8>,
    pub data: Vec<u8>,
    /// Seconds since 1970-01-01 UTC.
    pub time: u32,
}

/// Whether `kind` is an event type `ReportEvent` takes.
pub fn valid_type(kind: u16) -> bool {
    EVENT_TYPES.contains(&kind)
}

fn upcase(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| upcase(x) == upcase(y))
}

/// Whether `server` (`lpUNCServerName`, without its NUL) is this computer:
/// none, empty, its name (with or without the leading `\\`), `localhost` or
/// `.`. Any other server is `RPC_S_SERVER_UNAVAILABLE` (`RegisterEventSourceA.yaml`
/// `bad_args`: Poly reaches no other computer's log).
pub fn is_local_server(server: Option<&[u16]>, computer: &[u16]) -> bool {
    let Some(s) = server else { return true };
    let s = s.strip_prefix(&[u16::from(b'\\'), u16::from(b'\\')]).unwrap_or(s);
    let w = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
    s.is_empty() || eq_nocase(s, computer) || eq_nocase(s, &w("localhost")) || s == w(".").as_slice()
}

/// The log a name opens (`OpenEventLog`): System, else Application.
pub fn log_of(name: &[u16]) -> &'static str {
    let system: Vec<u16> = SYSTEM.encode_utf16().collect();
    if eq_nocase(name, &system) { SYSTEM } else { APPLICATION }
}

fn put_u16(out: &mut [u8], at: usize, v: u16) {
    out[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn put_text(out: &mut Vec<u8>, s: &[u16]) {
    for &c in s {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
}

fn align4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// `e` as an `EVENTLOGRECORD` numbered `number` (`winnt.h`'s layout; Learn
/// orders the variable part: source name, computer name, the user SID on a
/// 4-byte boundary, the strings, the data, padding to 4 bytes, the length
/// again). No closing record number (`ReportEventA.yaml` `success`: 0).
pub fn encode(e: &Event, number: u32) -> Vec<u8> {
    let mut out = alloc::vec![0u8; RECORD_HEADER];
    put_text(&mut out, &e.source);
    put_text(&mut out, &e.computer);
    align4(&mut out);
    let sid_offset = out.len();
    out.extend_from_slice(&e.sid);
    let string_offset = out.len();
    for s in &e.strings {
        put_text(&mut out, s);
    }
    let data_offset = out.len();
    out.extend_from_slice(&e.data);
    align4(&mut out);
    let length = out.len() + 4;
    out.extend_from_slice(&(length as u32).to_le_bytes());
    put_u32(&mut out, 0, length as u32);
    put_u32(&mut out, 4, ELF_LOG_SIGNATURE);
    put_u32(&mut out, 8, number);
    put_u32(&mut out, 12, e.time);
    put_u32(&mut out, 16, e.time);
    put_u32(&mut out, 20, e.event_id);
    put_u16(&mut out, 24, e.kind);
    put_u16(&mut out, 26, e.strings.len() as u16);
    put_u16(&mut out, 28, e.category);
    put_u32(&mut out, 36, string_offset as u32);
    put_u32(&mut out, 40, e.sid.len() as u32);
    put_u32(&mut out, 44, sid_offset as u32);
    put_u32(&mut out, 48, e.data.len() as u32);
    put_u32(&mut out, 52, data_offset as u32);
    out
}

/// The records of a log file, oldest first: each a well-formed
/// `EVENTLOGRECORD` (its length, signature and trailing length agree). The
/// file ends at the first one that is not (a write cut short).
pub fn records(file: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(len) = u32_at(file, at).map(|l| l as usize) {
        let ok = len >= RECORD_HEADER + 4
            && len.is_multiple_of(4)
            && at + len <= file.len()
            && u32_at(file, at + 4) == Some(ELF_LOG_SIGNATURE)
            && u32_at(file, at + len - 4) == Some(len as u32);
        if !ok {
            break;
        }
        out.push(&file[at..at + len]);
        at += len;
    }
    out
}

/// A record's number.
pub fn number_of(record: &[u8]) -> u32 {
    u32_at(record, 8).unwrap_or(0)
}

/// What a new record of a log file is numbered: one more than its newest.
pub fn next_number(file: &[u8]) -> u32 {
    records(file).last().map_or(1, |r| number_of(r).wrapping_add(1).max(1))
}

/// The length of the well-formed records at the start of a log file: where
/// the next record goes (anything after it is a cut write and is replaced).
pub fn end_of_records(file: &[u8]) -> usize {
    records(file).iter().map(|r| r.len()).sum()
}

/// A reading handle's position: the number of the record a sequential read
/// starts at, `None` before the first read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub next: Option<u32>,
}

/// `ReadEventLogW(hEventLog, dwReadFlags, dwRecordOffset, ...)` on log file
/// `file` with a buffer of `capacity` bytes (Learn: "ReadEventLogW"):
///
/// - The flags name exactly one of sequential and seek reading and exactly
///   one direction, else `ERROR_INVALID_PARAMETER`.
/// - A sequential read goes on from where the last one stopped (the first
///   starts at the oldest record forwards, the newest backwards); a seek read
///   starts at record `offset`.
/// - As many whole records as fit are returned, in the reading direction. If
///   not even the first fits: `Err((ERROR_INSUFFICIENT_BUFFER, its length))`
///   and the position stays. No record left: `Err((ERROR_HANDLE_EOF, 0))`.
pub fn read(file: &[u8], cursor: &mut Cursor, flags: u32, offset: u32, capacity: usize) -> Result<Vec<u8>, (u32, u32)> {
    let seq = flags & EVENTLOG_SEQUENTIAL_READ != 0;
    let seek = flags & EVENTLOG_SEEK_READ != 0;
    let fwd = flags & EVENTLOG_FORWARDS_READ != 0;
    let back = flags & EVENTLOG_BACKWARDS_READ != 0;
    if seq == seek || fwd == back || flags & !0xF != 0 {
        return Err((ERROR_INVALID_PARAMETER, 0));
    }
    let all = records(file);
    let start = if seek { Some(offset) } else { cursor.next };
    // The records in reading order from the start.
    let picked: Vec<&[u8]> = if back {
        all.iter().rev().filter(|r| start.is_none_or(|s| number_of(r) <= s)).copied().collect()
    } else {
        all.iter().filter(|r| start.is_none_or(|s| number_of(r) >= s)).copied().collect()
    };
    let Some(first) = picked.first() else { return Err((ERROR_HANDLE_EOF, 0)) };
    if first.len() > capacity {
        return Err((ERROR_INSUFFICIENT_BUFFER, first.len() as u32));
    }
    let mut out = Vec::new();
    let mut last = 0;
    for r in picked {
        if out.len() + r.len() > capacity {
            break;
        }
        out.extend_from_slice(r);
        last = number_of(r);
    }
    cursor.next = Some(if back { last.wrapping_sub(1) } else { last.wrapping_add(1) });
    if back && last <= 1 {
        // Nothing older is left: the next backwards read is at the end.
        cursor.next = Some(0);
    }
    Ok(out)
}

/// What an event log handle is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `RegisterEventSource`: writes as this source to the Application log.
    Source { name: Vec<u16> },
    /// `OpenEventLog`: reads this log.
    Reader { log: &'static str, cursor: Cursor },
}

/// The handles of one process. A handle value is never given out twice, so
/// a closed handle stays invalid (`RegisterEventSourceA.yaml` `deregister`).
#[derive(Clone, Debug, Default)]
pub struct Handles {
    issued: u64,
    entries: Vec<(usize, Entry)>,
}

/// The first handle value and the step between them.
const HANDLE_BASE: usize = 0x0045_0000;
const HANDLE_STEP: usize = 4;

impl Handles {
    pub const fn new() -> Handles {
        Handles { issued: 0, entries: Vec::new() }
    }

    pub fn add(&mut self, e: Entry) -> usize {
        self.issued += 1;
        let h = HANDLE_BASE + self.issued as usize * HANDLE_STEP;
        self.entries.push((h, e));
        h
    }

    pub fn get_mut(&mut self, h: usize) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|(k, _)| *k == h).map(|(_, e)| e)
    }

    pub fn remove(&mut self, h: usize) -> Option<Entry> {
        let at = self.entries.iter().position(|(k, _)| *k == h)?;
        Some(self.entries.remove(at).1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;

    fn event(id: u32, strings: &[&str], data: &[u8]) -> Event {
        Event {
            source: wide("PolySpecB4RepA"),
            computer: wide("POLY"),
            event_id: id,
            kind: 4,
            category: 7,
            strings: strings.iter().map(|s| wide(s)).collect(),
            sid: Vec::new(),
            data: data.to_vec(),
            time: 1_000_000,
        }
    }

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn text_at(b: &[u8], mut at: usize) -> (alloc::string::String, usize) {
        let mut units = Vec::new();
        while u16_at(b, at) != 0 {
            units.push(u16_at(b, at));
            at += 2;
        }
        (alloc::string::String::from_utf16(&units).unwrap(), at + 2)
    }

    #[test]
    fn record_layout() {
        // ReportEventA.yaml `success`: two strings, 5 data bytes, category 7.
        let r = encode(&event(0x1234_5678, &["hello one", "second"], &[1, 2, 3, 4, 5]), 9);
        let len = r.len();
        assert_eq!(len % 4, 0);
        assert_eq!(u32_at(&r, 0), Some(len as u32));
        assert_eq!(u32_at(&r, len - 4), Some(len as u32));
        assert_eq!(u32_at(&r, 4), Some(ELF_LOG_SIGNATURE));
        assert_eq!(number_of(&r), 9);
        assert_eq!(u32_at(&r, 20), Some(0x1234_5678));
        assert_eq!((u16_at(&r, 24), u16_at(&r, 26), u16_at(&r, 28)), (4, 2, 7));
        assert_eq!(u32_at(&r, 32), Some(0), "no closing record");
        assert_eq!(u32_at(&r, 40), Some(0), "no user SID");
        let (source, after) = text_at(&r, RECORD_HEADER);
        assert_eq!(source, "PolySpecB4RepA");
        assert_eq!(text_at(&r, after).0, "POLY");
        let so = u32_at(&r, 36).unwrap() as usize;
        let (s1, next) = text_at(&r, so);
        assert_eq!((s1.as_str(), text_at(&r, next).0.as_str()), ("hello one", "second"));
        let (dl, dof) = (u32_at(&r, 48).unwrap() as usize, u32_at(&r, 52).unwrap() as usize);
        assert_eq!(&r[dof..dof + dl], &[1, 2, 3, 4, 5]);
        // An empty string counts (one string, the empty one).
        let e = encode(&event(7, &[""], &[]), 1);
        assert_eq!(u16_at(&e, 26), 1);
        assert_eq!(text_at(&e, u32_at(&e, 36).unwrap() as usize).0, "");
    }

    #[test]
    fn types() {
        for t in [0, 1, 2, 4, 8, 16] {
            assert!(valid_type(t), "{t}");
        }
        for t in [3, 5, 0x20, 0x40, 0xFFFF] {
            assert!(!valid_type(t), "{t}");
        }
    }

    #[test]
    fn servers_and_logs() {
        let me = wide("POLY");
        assert!(is_local_server(None, &me));
        assert!(is_local_server(Some(&[]), &me));
        assert!(is_local_server(Some(&wide("\\\\poly")), &me));
        assert!(!is_local_server(Some(&wide("\\\\poly-no-such-computer-xyz")), &me));
        assert_eq!(log_of(&wide("system")), SYSTEM);
        assert_eq!(log_of(&wide("Application")), APPLICATION);
        assert_eq!(log_of(&wide("PolySpecB4NoSuchLog")), APPLICATION);
    }

    fn log_of_events(n: u32) -> Vec<u8> {
        let mut file = Vec::new();
        for i in 0..n {
            let number = next_number(&file);
            file.extend(encode(&event(0x100 + i, &["x"], &[]), number));
        }
        file
    }

    #[test]
    fn numbering_and_cut_writes() {
        assert_eq!(next_number(&[]), 1);
        let mut file = log_of_events(3);
        assert_eq!(records(&file).len(), 3);
        assert_eq!(next_number(&file), 4);
        let end = end_of_records(&file);
        file.extend_from_slice(&[0x40, 0, 0, 0, 1, 2]);
        assert_eq!(records(&file).len(), 3, "a cut record is not one");
        assert_eq!(end_of_records(&file), end);
    }

    #[test]
    fn reading_backwards_and_forwards() {
        let file = log_of_events(3);
        let mut c = Cursor::default();
        let back = EVENTLOG_SEQUENTIAL_READ | EVENTLOG_BACKWARDS_READ;
        // Newest first, all in one buffer, then the end.
        let got = read(&file, &mut c, back, 0, 32768).unwrap();
        let numbers: Vec<u32> = records(&got).iter().map(|r| number_of(r)).collect();
        assert_eq!(numbers, [3, 2, 1]);
        assert_eq!(read(&file, &mut c, back, 0, 32768), Err((ERROR_HANDLE_EOF, 0)));
        // One record at a time; a buffer too small says how much is needed.
        let one = records(&file)[0].len();
        let mut c = Cursor::default();
        assert_eq!(read(&file, &mut c, back, 0, 10), Err((ERROR_INSUFFICIENT_BUFFER, one as u32)));
        assert_eq!(number_of(&read(&file, &mut c, back, 0, one).unwrap()), 3);
        assert_eq!(number_of(&read(&file, &mut c, back, 0, one).unwrap()), 2);
        // Forwards from the oldest; seek to a record.
        let mut c = Cursor::default();
        let fwd = EVENTLOG_SEQUENTIAL_READ | EVENTLOG_FORWARDS_READ;
        assert_eq!(number_of(&read(&file, &mut c, fwd, 0, one).unwrap()), 1);
        let seek = EVENTLOG_SEEK_READ | EVENTLOG_FORWARDS_READ;
        assert_eq!(number_of(&read(&file, &mut c, seek, 3, 32768).unwrap()), 3);
        // An empty log, bad flags.
        assert_eq!(read(&[], &mut Cursor::default(), back, 0, 100), Err((ERROR_HANDLE_EOF, 0)));
        assert_eq!(read(&file, &mut c, 0x3 | 0x8, 0, 100), Err((ERROR_INVALID_PARAMETER, 0)));
        assert_eq!(read(&file, &mut c, 0x1, 0, 100), Err((ERROR_INVALID_PARAMETER, 0)));
    }

    #[test]
    fn handles_are_never_reused() {
        let mut h = Handles::new();
        let a = h.add(Entry::Source { name: wide("A") });
        let b = h.add(Entry::Source { name: wide("A") });
        assert_ne!(a, b);
        assert!(a != 0 && a != usize::MAX);
        assert!(h.remove(a).is_some());
        assert!(h.remove(a).is_none());
        let c = h.add(Entry::Reader { log: APPLICATION, cursor: Cursor::default() });
        assert!(c != a && c != b);
        assert!(h.get_mut(b).is_some());
    }
}
