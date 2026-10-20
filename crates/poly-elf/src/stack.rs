//! The initial process stack (System V AMD64 psABI §3.4.1, figure "Initial
//! Process Stack").
//!
//! From `rsp` upwards a new process finds:
//!
//! ```text
//!   rsp ->  argc
//!           argv[0] .. argv[argc - 1], 0
//!           envp[0] .. envp[n - 1], 0
//!           auxv: (type, value) pairs, ending with (AT_NULL, 0)
//!           padding (unspecified)
//!           information block: random bytes, copied program headers (if
//!           needed), the platform string, argument strings, environment
//!           strings, the executable's file name
//!           8 zero bytes
//!   stack top
//! ```
//!
//! `rsp` is 16-byte aligned, as the psABI requires at process entry. The
//! auxiliary vector types are those of the public `<elf.h>`; their meanings
//! are documented in getauxval(3).

use crate::header::{PHDR_SIZE, u64_at};
use crate::{LoadPlan, PAGE_SIZE};

/// End of the auxiliary vector (`<elf.h>` `AT_NULL`).
pub const AT_NULL: u64 = 0;
/// Entry to ignore (`<elf.h>` `AT_IGNORE`).
pub const AT_IGNORE: u64 = 1;
/// Address of the program's program header table (`<elf.h>` `AT_PHDR`).
pub const AT_PHDR: u64 = 3;
/// Size of one program header (`<elf.h>` `AT_PHENT`).
pub const AT_PHENT: u64 = 4;
/// Number of program headers (`<elf.h>` `AT_PHNUM`).
pub const AT_PHNUM: u64 = 5;
/// System page size (`<elf.h>` `AT_PAGESZ`).
pub const AT_PAGESZ: u64 = 6;
/// Base address of the program interpreter, 0 without one (`<elf.h>` `AT_BASE`).
pub const AT_BASE: u64 = 7;
/// Flags, unused on x86-64 (`<elf.h>` `AT_FLAGS`).
pub const AT_FLAGS: u64 = 8;
/// Entry point of the program, not of the interpreter (`<elf.h>` `AT_ENTRY`).
pub const AT_ENTRY: u64 = 9;
/// Real user ID (`<elf.h>` `AT_UID`).
pub const AT_UID: u64 = 11;
/// Effective user ID (`<elf.h>` `AT_EUID`).
pub const AT_EUID: u64 = 12;
/// Real group ID (`<elf.h>` `AT_GID`).
pub const AT_GID: u64 = 13;
/// Effective group ID (`<elf.h>` `AT_EGID`).
pub const AT_EGID: u64 = 14;
/// Address of a string naming the hardware platform (`<elf.h>` `AT_PLATFORM`).
pub const AT_PLATFORM: u64 = 15;
/// CPU feature bits (`<elf.h>` `AT_HWCAP`); on x86 those of CPUID leaf 1 EDX.
pub const AT_HWCAP: u64 = 16;
/// Clock ticks per second, as `times(2)` counts them (`<elf.h>` `AT_CLKTCK`).
pub const AT_CLKTCK: u64 = 17;
/// Non-zero if the program runs with elevated privileges (`<elf.h>` `AT_SECURE`).
pub const AT_SECURE: u64 = 23;
/// Address of 16 random bytes (`<elf.h>` `AT_RANDOM`).
pub const AT_RANDOM: u64 = 25;
/// More CPU feature bits (`<elf.h>` `AT_HWCAP2`).
pub const AT_HWCAP2: u64 = 26;
/// Address of the file name passed to `execve` (`<elf.h>` `AT_EXECFN`).
pub const AT_EXECFN: u64 = 31;
/// Address of the vDSO's ELF header (`<elf.h>` `AT_SYSINFO_EHDR`).
pub const AT_SYSINFO_EHDR: u64 = 33;
/// Minimal stack size for signal delivery (`<elf.h>` `AT_MINSIGSTKSZ`).
pub const AT_MINSIGSTKSZ: u64 = 51;

/// The `AT_PLATFORM` string of x86-64 processes (`uname -m` on Linux).
pub const PLATFORM_X86_64: &[u8] = b"x86_64";
/// Clock ticks per second reported in `AT_CLKTCK`: 100, the value
/// `sysconf(_SC_CLK_TCK)` returns on Linux (see times(2)).
pub const CLOCK_TICKS: u64 = 100;
/// Bytes at the very top of the stack that stay zero, so the information
/// block never ends exactly at the top.
const END_MARKER: usize = 8;

/// User and group IDs of the new process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Credentials {
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
}

impl Credentials {
    /// Whether real and effective IDs differ, as after exec of a set-user-ID
    /// or set-group-ID program; the default for `AT_SECURE`.
    pub const fn elevated(&self) -> bool {
        self.uid != self.euid || self.gid != self.egid
    }
}

/// The values of the auxiliary vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxInfo {
    /// `AT_PHDR`; `None` if no segment maps the table, in which case
    /// [`StackParams::phdr_table`] is copied to the stack and used instead.
    pub phdr: Option<u64>,
    pub phent: u64,
    pub phnum: u64,
    pub pagesz: u64,
    /// `AT_BASE`: the interpreter's load bias, 0 for a static executable.
    pub base: u64,
    pub flags: u64,
    /// `AT_ENTRY`: the main executable's entry point.
    pub entry: u64,
    pub creds: Credentials,
    pub secure: bool,
    pub hwcap: u64,
    /// `AT_HWCAP2`, omitted when `None`.
    pub hwcap2: Option<u64>,
    pub clktck: u64,
    /// `AT_SYSINFO_EHDR`, omitted when `None` (no vDSO yet).
    pub sysinfo_ehdr: Option<u64>,
}

impl AuxInfo {
    /// Values for a process running `exe`, through `interp` if the
    /// executable names one. CPU feature bits start empty; set `hwcap`
    /// from CPUID.
    pub const fn new(exe: &LoadPlan, interp: Option<&LoadPlan>, creds: Credentials) -> AuxInfo {
        AuxInfo {
            phdr: exe.phdr(),
            phent: exe.phent() as u64,
            phnum: exe.phnum() as u64,
            pagesz: PAGE_SIZE,
            base: match interp {
                Some(i) => i.bias(),
                None => 0,
            },
            flags: 0,
            entry: exe.entry(),
            creds,
            secure: creds.elevated(),
            hwcap: 0,
            hwcap2: None,
            clktck: CLOCK_TICKS,
            sysinfo_ehdr: None,
        }
    }
}

/// Where execution starts: the interpreter's entry point if there is one,
/// else the executable's.
pub const fn start_address(exe: &LoadPlan, interp: Option<&LoadPlan>) -> u64 {
    match interp {
        Some(i) => i.entry(),
        None => exe.entry(),
    }
}

/// Everything that goes onto the initial stack.
#[derive(Clone, Copy, Debug)]
pub struct StackParams<'a> {
    /// Argument strings, without their terminating NUL.
    pub argv: &'a [&'a [u8]],
    /// Environment strings (`NAME=value`), without their terminating NUL.
    pub envp: &'a [&'a [u8]],
    /// The path passed to `execve`, for `AT_EXECFN`.
    pub execfn: &'a [u8],
    /// `AT_PLATFORM` string; omitted when empty. Normally [`PLATFORM_X86_64`].
    pub platform: &'a [u8],
    /// The 16 bytes `AT_RANDOM` points to; the C library seeds its stack
    /// protector and pointer guard from them, so they must be unpredictable.
    pub random: &'a [u8; 16],
    pub aux: AuxInfo,
    /// The program header table to copy onto the stack when `aux.phdr` is
    /// `None` ([`Elf::program_header_table`](crate::Elf::program_header_table)).
    pub phdr_table: &'a [u8],
    /// More auxiliary vector entries, placed before `AT_NULL` (for example
    /// `AT_MINSIGSTKSZ`). None may be `AT_NULL`.
    pub extra_aux: &'a [(u64, u64)],
}

/// Why the stack could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    /// The buffer is too small for everything.
    TooSmall,
    /// The buffer would extend below address 0.
    AddressOverflow,
    /// A string contains a NUL byte.
    InteriorNul,
    /// An extra auxiliary vector entry has type `AT_NULL`.
    BadAuxEntry,
    /// `aux.phdr` is `None` and `phdr_table` is empty or not a whole number
    /// of program headers.
    MissingPhdr,
}

/// Where things ended up on the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackLayout {
    /// Initial stack pointer (16-byte aligned); `argc` is stored here.
    pub rsp: u64,
    pub argc: u64,
    /// Address of `argv[0]` (`rsp + 8`).
    pub argv: u64,
    /// Address of `envp[0]`.
    pub envp: u64,
    /// Address of the first auxiliary vector entry.
    pub auxv: u64,
    /// Number of auxiliary vector entries, `AT_NULL` included.
    pub auxc: u64,
    /// Value of `AT_RANDOM`.
    pub random: u64,
    /// Value of `AT_EXECFN`.
    pub execfn: u64,
    /// Value of `AT_PHDR`.
    pub phdr: u64,
}

impl StackLayout {
    /// Bytes from `rsp` to the top of the stack.
    pub const fn used(&self, stack_top: u64) -> u64 {
        stack_top - self.rsp
    }
}

/// Writes downwards from the end of a buffer standing for the memory just
/// below `top`.
struct Down<'b> {
    buf: &'b mut [u8],
    /// Address of `buf[0]`.
    base: u64,
    /// Index of the lowest byte written so far.
    pos: usize,
}

impl Down<'_> {
    fn addr(&self) -> u64 {
        self.base + self.pos as u64
    }

    /// Writes `bytes` so that they end at the current position and start at
    /// a multiple of `align` (a power of two), zeroing the gap above them.
    fn push(&mut self, bytes: &[u8], align: u64) -> Result<u64, StackError> {
        let lowest = self.addr().checked_sub(bytes.len() as u64).ok_or(StackError::TooSmall)?;
        let at = lowest & !(align - 1);
        if at < self.base {
            return Err(StackError::TooSmall);
        }
        let from = (at - self.base) as usize;
        let dst = self.buf.get_mut(from..self.pos).ok_or(StackError::TooSmall)?;
        let (head, gap) = dst.split_at_mut(bytes.len());
        head.copy_from_slice(bytes);
        gap.fill(0);
        self.pos = from;
        Ok(at)
    }

    /// Writes a NUL-terminated copy of `s`.
    fn push_str(&mut self, s: &[u8]) -> Result<u64, StackError> {
        self.push(&[0], 1)?;
        self.push(s, 1)
    }
}

/// Builds the initial stack of a new process.
///
/// `buf` stands for the `buf.len()` bytes of the stack just below
/// `stack_top`: `buf[i]` is the byte at `stack_top - buf.len() + i`. Bytes
/// from the returned `rsp` up to `stack_top` are written (copy them, or
/// `buf` is the mapped stack itself); bytes below `rsp` are left alone.
pub fn build_initial_stack(buf: &mut [u8], stack_top: u64, p: &StackParams<'_>) -> Result<StackLayout, StackError> {
    let base = stack_top.checked_sub(buf.len() as u64).ok_or(StackError::AddressOverflow)?;
    if p.extra_aux.iter().any(|&(ty, _)| ty == AT_NULL) {
        return Err(StackError::BadAuxEntry);
    }
    let strings = p.argv.iter().chain(p.envp).chain([&p.execfn, &p.platform]);
    if strings.into_iter().any(|s| s.contains(&0)) {
        return Err(StackError::InteriorNul);
    }
    let pos = buf.len();
    let mut down = Down { buf, base, pos };

    // Information block, top down. Argument and environment strings are
    // pushed last to first so that they end up in ascending order.
    down.push(&[0; END_MARKER], 1)?;
    let execfn = down.push_str(p.execfn)?;
    for s in p.envp.iter().rev() {
        down.push_str(s)?;
    }
    let env_strings = down.addr();
    for s in p.argv.iter().rev() {
        down.push_str(s)?;
    }
    let arg_strings = down.addr();
    let platform = if p.platform.is_empty() { None } else { Some(down.push_str(p.platform)?) };
    let phdr = match p.aux.phdr {
        Some(addr) => addr,
        None => {
            if p.phdr_table.is_empty() || !p.phdr_table.len().is_multiple_of(PHDR_SIZE) {
                return Err(StackError::MissingPhdr);
            }
            down.push(p.phdr_table, 8)?
        }
    };
    let random = down.push(p.random, 16)?;

    // The auxiliary vector.
    let a = &p.aux;
    let optional = |ty, v: Option<u64>| v.map(|v| (ty, v));
    let fixed = [
        (AT_HWCAP, a.hwcap),
        (AT_PAGESZ, a.pagesz),
        (AT_CLKTCK, a.clktck),
        (AT_PHDR, phdr),
        (AT_PHENT, a.phent),
        (AT_PHNUM, a.phnum),
        (AT_BASE, a.base),
        (AT_FLAGS, a.flags),
        (AT_ENTRY, a.entry),
        (AT_UID, a.creds.uid.into()),
        (AT_EUID, a.creds.euid.into()),
        (AT_GID, a.creds.gid.into()),
        (AT_EGID, a.creds.egid.into()),
        (AT_SECURE, a.secure.into()),
        (AT_RANDOM, random),
    ];
    let auxv = optional(AT_SYSINFO_EHDR, a.sysinfo_ehdr)
        .into_iter()
        .chain(fixed)
        .chain(optional(AT_HWCAP2, a.hwcap2))
        .chain([(AT_EXECFN, execfn)])
        .chain(optional(AT_PLATFORM, platform))
        .chain(p.extra_aux.iter().copied())
        .chain([(AT_NULL, 0)]);

    // The vectors: argc, argv + NULL, envp + NULL, auxv pairs.
    let argc = p.argv.len() as u64;
    let envc = p.envp.len() as u64;
    let auxc = auxv.clone().count() as u64;
    let words = (argc + 1)
        .checked_add(envc + 1)
        .and_then(|w| w.checked_add(1))
        .and_then(|w| w.checked_add(auxc.checked_mul(2)?))
        .ok_or(StackError::TooSmall)?;
    let bytes = words.checked_mul(8).ok_or(StackError::TooSmall)?;
    let rsp = down.addr().checked_sub(bytes).ok_or(StackError::TooSmall)? & !15;
    if rsp < base {
        return Err(StackError::TooSmall);
    }
    let from = (rsp - base) as usize;
    let block = down.buf.get_mut(from..down.pos).ok_or(StackError::TooSmall)?;
    block.fill(0);
    let mut words = block.chunks_exact_mut(8);
    let mut put = |v: u64| {
        if let Some(w) = words.next() {
            w.copy_from_slice(&v.to_le_bytes());
        }
    };
    put(argc);
    let mut s = arg_strings;
    for arg in p.argv {
        put(s);
        s += arg.len() as u64 + 1;
    }
    put(0);
    let mut s = env_strings;
    for env in p.envp {
        put(s);
        s += env.len() as u64 + 1;
    }
    put(0);
    for (ty, v) in auxv {
        put(ty);
        put(v);
    }

    let argv = rsp + 8;
    let envp = argv + 8 * (argc + 1);
    Ok(StackLayout { rsp, argc, argv, envp, auxv: envp + 8 * (envc + 1), auxc, random, execfn, phdr })
}

/// Reads an initial stack back, for tests and for `/proc/<pid>/auxv`-style
/// introspection. Every read is bounds-checked against the buffer.
#[derive(Clone, Copy, Debug)]
pub struct InitialStack<'b> {
    buf: &'b [u8],
    base: u64,
    argc: u64,
    argv: u64,
    envp: u64,
    envc: u64,
    auxv: u64,
}

impl<'b> InitialStack<'b> {
    /// Decodes the stack at `rsp`, `buf` standing for the bytes just below
    /// `stack_top`. `None` if the vectors run off the buffer or are not
    /// terminated.
    pub fn parse(buf: &'b [u8], stack_top: u64, rsp: u64) -> Option<InitialStack<'b>> {
        let base = stack_top.checked_sub(buf.len() as u64)?;
        let mut s = InitialStack { buf, base, argc: 0, argv: 0, envp: 0, envc: 0, auxv: 0 };
        s.argc = s.word(rsp)?;
        s.argv = rsp.checked_add(8)?;
        let argv_end = s.argv.checked_add(s.argc.checked_mul(8)?)?;
        if s.word(argv_end)? != 0 {
            return None;
        }
        s.envp = argv_end + 8;
        let mut at = s.envp;
        while s.word(at)? != 0 {
            at = at.checked_add(8)?;
            s.envc += 1;
        }
        s.auxv = at.checked_add(8)?;
        // The auxiliary vector must end with AT_NULL inside the buffer.
        let mut at = s.auxv;
        while s.word(at)? != AT_NULL {
            at = at.checked_add(16)?;
        }
        s.word(at.checked_add(8)?)?;
        Some(s)
    }

    /// The little-endian word at `addr`.
    pub fn word(&self, addr: u64) -> Option<u64> {
        let off = usize::try_from(addr.checked_sub(self.base)?).ok()?;
        u64_at(self.buf, off)
    }

    /// The NUL-terminated string at `addr`, without the NUL.
    pub fn c_str(&self, addr: u64) -> Option<&'b [u8]> {
        let off = usize::try_from(addr.checked_sub(self.base)?).ok()?;
        let rest = self.buf.get(off..)?;
        rest.get(..rest.iter().position(|&b| b == 0)?)
    }

    pub const fn argc(&self) -> u64 {
        self.argc
    }

    pub const fn envc(&self) -> u64 {
        self.envc
    }

    /// `argv[i]`, if `i < argc`.
    pub fn arg(&self, i: u64) -> Option<&'b [u8]> {
        if i >= self.argc {
            return None;
        }
        self.c_str(self.word(self.argv + 8 * i)?)
    }

    /// `envp[i]`, if `i < envc`.
    pub fn env(&self, i: u64) -> Option<&'b [u8]> {
        if i >= self.envc {
            return None;
        }
        self.c_str(self.word(self.envp + 8 * i)?)
    }

    /// Address of `envp[0]`.
    pub const fn envp(&self) -> u64 {
        self.envp
    }

    /// Address of the first auxiliary vector entry.
    pub const fn auxv_addr(&self) -> u64 {
        self.auxv
    }

    /// The auxiliary vector entries before `AT_NULL`.
    pub fn auxv(&self) -> impl Iterator<Item = (u64, u64)> + '_ {
        let mut at = self.auxv;
        core::iter::from_fn(move || {
            let ty = self.word(at)?;
            let value = self.word(at.checked_add(8)?)?;
            at = at.checked_add(16)?;
            (ty != AT_NULL).then_some((ty, value))
        })
    }

    /// The value of auxiliary vector entry `ty`, like getauxval(3).
    pub fn aux(&self, ty: u64) -> Option<u64> {
        self.auxv().find(|&(t, _)| t == ty).map(|(_, v)| v)
    }
}
