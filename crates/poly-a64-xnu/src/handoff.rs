//! The hand-off block: what the kernel tells the runtime when it starts a
//! translated process (the runtime's first argument, `rdi`, is its address).
//!
//! Little-endian 64-bit words: a magic word, the version, the block's
//! length in bytes, the guest's first registers, the window and its parts,
//! the runtime's heap and code area, the engine to use, then the guest's
//! regions (address, length, protection) and the program's path.

use alloc::string::String;
use alloc::vec::Vec;

/// `POLYA64` and a 0 byte.
pub const MAGIC: u64 = u64::from_le_bytes(*b"POLYA64\0");
pub const VERSION: u64 = 2;
/// Words before the regions.
const FIXED_WORDS: usize = 18;
/// The most regions a block may list.
pub const MAX_REGIONS: usize = 1024;

/// What runs the guest's code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineKind {
    /// The block JIT, with the interpreter for what it leaves out.
    #[default]
    Jit,
    /// Only the interpreter.
    Interpreter,
}

/// Guest memory the kernel mapped: `prot` is what the guest may do there
/// (`poly_a64_jit::mem::{R, W, X}`); the host can read and write all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub addr: u64,
    pub len: u64,
    pub prot: u8,
}

/// An image's thread-local variables, for the blocks of new threads: the
/// thread's table has the block at `slot`, a copy of `init_size` bytes of
/// the template at `template` followed by zeros up to `size`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TlvImage {
    pub slot: u64,
    pub template: u64,
    pub init_size: u64,
    pub size: u64,
}

/// The hand-off block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Handoff {
    /// The guest's first program counter and stack pointer.
    pub pc: u64,
    pub sp: u64,
    /// `TPIDRRO_EL0`: the main thread's TSD page.
    pub tpidrro: u64,
    /// The window (see [`crate::layout`]).
    pub window_lo: u64,
    pub window_end: u64,
    /// Where the runtime places the guest's `mmap` mappings.
    pub mmap_lo: u64,
    pub mmap_end: u64,
    /// The runtime's own heap and code area.
    pub heap: u64,
    pub heap_size: u64,
    pub code: u64,
    pub code_size: u64,
    pub engine: EngineKind,
    /// Print counters and the time taken when the guest exits.
    pub stats: bool,
    /// Also count what the interpreter runs, by kind, and print the most
    /// frequent with the counters.
    pub profile: bool,
    /// The guest's mapped memory.
    pub regions: Vec<Region>,
    /// The program's path (for messages).
    pub path: String,
    /// The images with thread-local variables.
    pub tlv: Vec<TlvImage>,
}

/// The environment variable that picks the engine: `interp` for the
/// interpreter alone, anything else for the JIT.
pub const ENGINE_VAR: &[u8] = b"POLY_A64_ENGINE";
/// The environment variable that turns the exit statistics on (`1`).
pub const STATS_VAR: &[u8] = b"POLY_A64_STATS";
/// The environment variable that turns the interpreter profile on (`1`;
/// implies the statistics).
pub const PROFILE_VAR: &[u8] = b"POLY_A64_PROFILE";

/// Whether the environment asks for the interpreter profile.
pub fn profile_from_env(envp: &[&[u8]]) -> bool {
    envp.iter().any(|e| e.strip_prefix(PROFILE_VAR).and_then(|r| r.strip_prefix(b"=")) == Some(b"1"))
}

/// The engine and statistics settings from a program's environment.
pub fn settings_from_env(envp: &[&[u8]]) -> (EngineKind, bool) {
    let value = |name: &[u8]| envp.iter().find_map(|e| e.strip_prefix(name).and_then(|rest| rest.strip_prefix(b"=")));
    let engine = match value(ENGINE_VAR) {
        Some(b"interp") => EngineKind::Interpreter,
        _ => EngineKind::Jit,
    };
    (engine, value(STATS_VAR).is_some_and(|v| v == b"1"))
}

impl Handoff {
    /// The block's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let path = self.path.as_bytes();
        let words = FIXED_WORDS + 3 * self.regions.len() + 1 + 4 * self.tlv.len() + path.len().div_ceil(8);
        let len = (words * 8) as u64;
        let mut w: Vec<u64> = Vec::with_capacity(words);
        w.extend([
            MAGIC,
            VERSION,
            len,
            self.pc,
            self.sp,
            self.tpidrro,
            self.window_lo,
            self.window_end,
            self.mmap_lo,
            self.mmap_end,
            self.heap,
            self.heap_size,
            self.code,
            self.code_size,
            match self.engine {
                EngineKind::Jit => 0,
                EngineKind::Interpreter => 1,
            },
            u64::from(self.stats) | u64::from(self.profile) << 1,
            path.len() as u64,
            self.regions.len() as u64,
        ]);
        for r in &self.regions {
            w.extend([r.addr, r.len, u64::from(r.prot)]);
        }
        w.push(self.tlv.len() as u64);
        for t in &self.tlv {
            w.extend([t.slot, t.template, t.init_size, t.size]);
        }
        let mut out: Vec<u8> = w.iter().flat_map(|v| v.to_le_bytes()).collect();
        out.extend_from_slice(path);
        out.resize(len as usize, 0);
        out
    }

    /// The length a block says it has, from its first three words (`None`
    /// if they are not a hand-off block's).
    pub fn length(head: &[u8; 24]) -> Option<usize> {
        let w = |i: usize| u64::from_le_bytes(head[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
        (w(0) == MAGIC && w(1) == VERSION).then(|| usize::try_from(w(2)).ok()).flatten()
    }

    /// Read a block; `None` if it is not a whole, valid one.
    pub fn from_bytes(b: &[u8]) -> Option<Handoff> {
        let w = |i: usize| -> Option<u64> { Some(u64::from_le_bytes(b.get(i * 8..i * 8 + 8)?.try_into().ok()?)) };
        if w(0)? != MAGIC || w(1)? != VERSION || w(2)? != b.len() as u64 {
            return None;
        }
        let path_len = usize::try_from(w(16)?).ok()?;
        let count = usize::try_from(w(17)?).ok()?;
        if count > MAX_REGIONS {
            return None;
        }
        let mut regions = Vec::with_capacity(count);
        for i in 0..count {
            let at = FIXED_WORDS + 3 * i;
            regions.push(Region { addr: w(at)?, len: w(at + 1)?, prot: u8::try_from(w(at + 2)?).ok()? });
        }
        let at = FIXED_WORDS + 3 * count;
        let ntlv = usize::try_from(w(at)?).ok()?;
        if ntlv > MAX_REGIONS {
            return None;
        }
        let mut tlv = Vec::with_capacity(ntlv);
        for i in 0..ntlv {
            let o = at + 1 + 4 * i;
            tlv.push(TlvImage { slot: w(o)?, template: w(o + 1)?, init_size: w(o + 2)?, size: w(o + 3)? });
        }
        let from = (at + 1 + 4 * ntlv) * 8;
        let path = b.get(from..from.checked_add(path_len)?)?;
        Some(Handoff {
            pc: w(3)?,
            sp: w(4)?,
            tpidrro: w(5)?,
            window_lo: w(6)?,
            window_end: w(7)?,
            mmap_lo: w(8)?,
            mmap_end: w(9)?,
            heap: w(10)?,
            heap_size: w(11)?,
            code: w(12)?,
            code_size: w(13)?,
            engine: match w(14)? {
                0 => EngineKind::Jit,
                1 => EngineKind::Interpreter,
                _ => return None,
            },
            stats: w(15)? & 1 != 0,
            profile: w(15)? & 2 != 0,
            regions,
            path: String::from(core::str::from_utf8(path).ok()?),
            tlv,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> Handoff {
        Handoff {
            pc: 0x1_0000_4000,
            sp: 0xF_FEFF_FF00,
            tpidrro: 0xF_0001_0000,
            window_lo: 0x1_0000_0000,
            window_end: 0x10_0000_0000,
            mmap_lo: 0xC_0000_0000,
            mmap_end: 0xC_8000_0000,
            heap: 0x100_0000_0000,
            heap_size: 1 << 28,
            code: 0x100_4000_0000,
            code_size: 1 << 25,
            engine: EngineKind::Interpreter,
            stats: true,
            profile: true,
            regions: vec![Region { addr: 0x1_0000_0000, len: 0x4000, prot: 5 }, Region { addr: 2, len: 3, prot: 3 }],
            path: String::from("/bin/a64-hello"),
            tlv: vec![TlvImage { slot: 1, template: 0x1_0000_8000, init_size: 4, size: 16 }],
        }
    }

    #[test]
    fn round_trip() {
        let h = sample();
        let b = h.to_bytes();
        assert_eq!(b.len() % 8, 0);
        assert_eq!(Handoff::length(b[..24].try_into().unwrap()), Some(b.len()));
        assert_eq!(Handoff::from_bytes(&b), Some(h));
        let empty = Handoff::default();
        assert_eq!(Handoff::from_bytes(&empty.to_bytes()), Some(empty));
    }

    #[test]
    fn refuses_damaged_blocks() {
        let b = sample().to_bytes();
        assert_eq!(Handoff::from_bytes(&b[..b.len() - 8]), None);
        for i in 0..b.len() {
            let mut c = b.clone();
            c[i] ^= 0xff;
            // Never panics; a changed header word is refused.
            let r = Handoff::from_bytes(&c);
            if i < 24 {
                assert_eq!(r, None);
            }
        }
        assert_eq!(Handoff::length(&[0; 24]), None);
    }

    #[test]
    fn settings() {
        assert_eq!(settings_from_env(&[]), (EngineKind::Jit, false));
        assert_eq!(
            settings_from_env(&[b"POLY_A64_ENGINE=interp", b"POLY_A64_STATS=1", b"X=1"]),
            (EngineKind::Interpreter, true)
        );
        assert_eq!(settings_from_env(&[b"POLY_A64_ENGINE=jit", b"POLY_A64_STATSX=1"]), (EngineKind::Jit, false));
        // The first one counts.
        assert_eq!(
            settings_from_env(&[b"POLY_A64_ENGINE=interp", b"POLY_A64_ENGINE=jit"]),
            (EngineKind::Interpreter, false)
        );
    }
}
