//! personality(2): the calling thread's execution domain and the flags
//! that change how some calls behave for it.
//!
//! The persona is a 32-bit value kept per thread
//! ([`LinuxThread::personality`](crate::signal::LinuxThread::personality)),
//! copied to the threads and processes it creates. Every value is stored
//! as given; the flags Poly acts on (personality's spec sheet):
//!
//! - `PER_LINUX32`: uname(2) reports the machine as `i686`;
//! - `UNAME26`: uname(2) reports the release as a `2.6.` one;
//! - `READ_IMPLIES_EXEC`: mmap(2) and mprotect(2) with `PROT_READ` also
//!   make the pages executable; execve(2) of an x86-64 program clears it
//!   (Poly runs only those, and they never ask for an executable stack
//!   that would keep it).
//!
//! Constants from `linux/personality.h`.

use crate::args::Args;
use crate::errno::SysResult;
use crate::kernel::LinuxKernel;
use crate::signal::deliver::with_own;
use crate::table::{Handler, nr};
use alloc::vec::Vec;

pub const PER_LINUX: u32 = 0x0000;
pub const PER_LINUX32: u32 = 0x0008;
pub const PER_MASK: u32 = 0x00ff;
pub const UNAME26: u32 = 0x0020000;
pub const ADDR_NO_RANDOMIZE: u32 = 0x0040000;
pub const READ_IMPLIES_EXEC: u32 = 0x0400000;
/// The argument that only reads the persona.
pub const QUERY: u32 = 0xffff_ffff;

pub const CALLS: &[(u64, Handler)] = &[(nr::PERSONALITY, personality)];

/// The calling thread's persona.
pub fn current(k: &mut dyn LinuxKernel) -> u32 {
    let tid = k.tid();
    with_own(k, |p| p.thread(tid).personality)
}

/// personality(persona): the old persona, zero-extended; unless the low
/// 32 bits of `persona` are `0xffffffff`, they become the new one. Never
/// fails.
pub fn personality(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let new = a.uint(0);
    let tid = k.tid();
    let old = with_own(k, |p| {
        let t = p.thread_mut(tid);
        let old = t.personality;
        if new != QUERY {
            t.personality = new;
        }
        old
    });
    Ok(u64::from(old))
}

/// The persona after execve(2): `READ_IMPLIES_EXEC` goes (the program
/// does not ask for an executable stack), the rest stays.
pub const fn after_exec(persona: u32) -> u32 {
    persona & !READ_IMPLIES_EXEC
}

/// The protection a mapping gets under `persona`: with
/// `READ_IMPLIES_EXEC`, readable means executable too.
pub const fn effective_prot(persona: u32, prot: u32) -> u32 {
    use crate::abi::prot::{PROT_EXEC, PROT_READ};
    if persona & READ_IMPLIES_EXEC != 0 && prot & PROT_READ != 0 { prot | PROT_EXEC } else { prot }
}

/// The machine uname(2) reports under `persona`.
pub fn uname_machine(persona: u32, machine: &'static str) -> &'static str {
    if persona & PER_MASK == PER_LINUX32 { "i686" } else { machine }
}

/// The release uname(2) reports under `persona`: with `UNAME26`, the
/// leading "major.minor.patch" of `release` becomes "2.6.<60 + minor>"
/// (6.18.54.1 reads as 2.6.78.1, personality's spec sheet).
pub fn uname_release(persona: u32, release: &str) -> Vec<u8> {
    if persona & UNAME26 == 0 {
        return release.as_bytes().to_vec();
    }
    let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
    let major_len = digits(release);
    let rest = &release[major_len..];
    let Some(after_major) = rest.strip_prefix('.') else { return release.as_bytes().to_vec() };
    let minor_len = digits(after_major);
    let minor: u32 = after_major[..minor_len].parse().unwrap_or(0);
    let mut tail = &after_major[minor_len..];
    // The patch level goes.
    if let Some(after_patch) = tail.strip_prefix('.') {
        tail = &after_patch[digits(after_patch)..];
    }
    let mut out = alloc::format!("2.6.{}", 60 + minor).into_bytes();
    out.extend_from_slice(tail.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::prot::{PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE};

    #[test]
    fn exec_clears_only_read_implies_exec() {
        assert_eq!(after_exec(READ_IMPLIES_EXEC | ADDR_NO_RANDOMIZE | PER_LINUX32), ADDR_NO_RANDOMIZE | PER_LINUX32);
        assert_eq!(after_exec(0x0800_0000 | 6), 0x0800_0000 | 6);
    }

    #[test]
    fn read_implies_exec() {
        assert_eq!(effective_prot(READ_IMPLIES_EXEC, PROT_READ), PROT_READ | PROT_EXEC);
        assert_eq!(effective_prot(READ_IMPLIES_EXEC, PROT_READ | PROT_WRITE), PROT_READ | PROT_WRITE | PROT_EXEC);
        assert_eq!(effective_prot(READ_IMPLIES_EXEC, PROT_NONE), PROT_NONE);
        assert_eq!(effective_prot(0, PROT_READ), PROT_READ);
    }

    #[test]
    fn uname_flags() {
        assert_eq!(uname_machine(PER_LINUX32, "x86_64"), "i686");
        assert_eq!(uname_machine(PER_LINUX32 | UNAME26, "x86_64"), "i686");
        assert_eq!(uname_machine(0, "x86_64"), "x86_64");
        assert_eq!(uname_release(UNAME26, "6.18.54.1-microsoft"), b"2.6.78.1-microsoft");
        assert_eq!(uname_release(UNAME26, "6.1.0-poly"), b"2.6.61-poly");
        assert_eq!(uname_release(0, "6.1.0-poly"), b"6.1.0-poly");
    }
}
