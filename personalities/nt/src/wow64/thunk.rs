//! The system calls of a 32-bit process (ADR 0186): read the caller's 4-byte arguments, widen
//! them by the service's row in [`super::sigs`], run the ordinary 64-bit service against a
//! [`Thunked`] kernel, and write the results back in the 32-bit layout.
//!
//! The 64-bit copies of the call's structures live in a **scratch window** at [`SCRATCH_BASE`],
//! above 4 GiB where nothing of a 32-bit process is mapped. [`Thunked`] serves the services'
//! reads and writes of that window from a buffer that lives for the one call, and passes every
//! other address through to the caller's memory. The arguments past the fourth are read by the
//! services from `RSP + 0x28 + 8 * (n - 4)` (the x64 convention, `crate::abi`), so the call's
//! RSP points at the window's start and those slots are the window's first bytes.
//!
//! A service that keeps an address for later (an asynchronous request's `IO_STATUS_BLOCK`, an
//! APC routine) would keep a window address that means nothing after the call: such arguments
//! are `X` in the table, or (the status block) only correct for calls that finish in the call,
//! which is every call on a synchronous handle. ADR 0186 lists this as a later stage.

use super::sigs;
use crate::kernel::{Apc, NtKernel, NtTimeout, ProcessCreate, ProcessCreated, ThreadCreate, ThreadCreated, UserFault};
use crate::services::Service;
use crate::system::{NtProcess, NtSystem};
use crate::{Backend, NtContext, abi};
use alloc::vec::Vec;
use poly_abi::{Outcome, SyscallRegs};
use poly_ntobj::status::{STATUS_INFO_LENGTH_MISMATCH, STATUS_NO_MEMORY, STATUS_NOT_IMPLEMENTED};
use poly_ntobj::{Caller, NtStatus, ObjectId, ThreadRef, WaitId};

/// How a 32-bit argument becomes a 64-bit one (see [`super::sigs`] for the table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum Arg {
    /// A value, zero extended.
    V,
    /// A handle, sign extended.
    H,
    /// A pointer to data with the same layout in both widths.
    P,
    /// `HANDLE*`.
    OH,
    /// `SIZE_T*`, `PVOID*`, `ULONG_PTR*`.
    OS,
    /// `OBJECT_ATTRIBUTES*`.
    OA,
    /// `UNICODE_STRING*`, in.
    US,
    /// `UNICODE_STRING*`, in and out.
    UO,
    /// `IO_STATUS_BLOCK*`.
    IO,
    /// `CLIENT_ID*`.
    CI,
    /// A `HANDLE` array counted by the first argument.
    HA,
    /// `CONTEXT*` (in).
    CX,
    /// `EXCEPTION_RECORD*` (in).
    ER,
    /// Not converted yet: must be null.
    X,
}

/// Where the scratch window starts: above 4 GiB, canonical, far from anything a 64-bit process
/// maps by default (and nothing at all is mapped above 4 GiB in a 32-bit one).
pub const SCRATCH_BASE: u64 = 0x0000_7000_0000_0000;
/// The most bytes a call's 64-bit copies take (16 arguments, a few structures and names).
const SCRATCH_MAX: usize = 0x2_0000;
/// The bytes of the argument area at the window's start: the 0x28 bytes below the fifth
/// argument and 8 per argument from the fifth on (14 arguments at most).
const ARG_AREA: usize = 0x28 + 8 * 12;

/// The 64-bit copies of one call's structures.
#[derive(Debug, Default)]
pub struct Scratch {
    bytes: Vec<u8>,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch { bytes: alloc::vec![0u8; ARG_AREA] }
    }

    /// Room for `len` zero bytes, 16-byte aligned; its address.
    fn alloc(&mut self, len: usize) -> Result<u64, NtStatus> {
        let at = self.bytes.len().next_multiple_of(16);
        if at + len > SCRATCH_MAX {
            return Err(STATUS_NO_MEMORY);
        }
        self.bytes.resize(at + len, 0);
        Ok(SCRATCH_BASE + at as u64)
    }

    /// `len` bytes at window address `addr`, if they are all in the window.
    fn range(&self, addr: u64, len: usize) -> Option<core::ops::Range<usize>> {
        let off = usize::try_from(addr.checked_sub(SCRATCH_BASE)?).ok()?;
        let end = off.checked_add(len)?;
        (end <= self.bytes.len()).then_some(off..end)
    }

    fn put(&mut self, addr: u64, data: &[u8]) {
        let r = self.range(addr, data.len()).expect("inside the window");
        self.bytes[r].copy_from_slice(data);
    }

    fn get(&self, addr: u64, len: usize) -> &[u8] {
        &self.bytes[self.range(addr, len).expect("inside the window")]
    }

    fn u64_at(&self, addr: u64) -> u64 {
        u64::from_le_bytes(self.get(addr, 8).try_into().expect("8 bytes"))
    }

    /// Whether `addr` falls in the window's address range at all (allocated or not).
    fn claims(addr: u64) -> bool {
        (SCRATCH_BASE..SCRATCH_BASE + SCRATCH_MAX as u64).contains(&addr)
    }
}

/// What to write back to the 32-bit caller once the service ran.
#[derive(Clone, Copy, Debug)]
enum Back {
    /// The low 32 bits of the 8 bytes at `wide`, to `narrow`.
    Low32 { narrow: u64, wide: u64 },
    /// An `IO_STATUS_BLOCK`: `Status` and `Information`.
    Iosb { narrow: u64, wide: u64 },
    /// A `UNICODE_STRING`'s `Length` and `MaximumLength`.
    Lengths { narrow: u64, wide: u64 },
}

/// An information class whose output has pointer-sized fields: the 64-bit size the service
/// writes, the 32-bit size the caller gets, and the conversion.
#[derive(Debug)]
struct InfoClass {
    class: u32,
    wide: usize,
    narrow: usize,
    convert: fn(&[u8], &mut [u8], &Facts),
}

/// What a conversion may need to know about the process.
#[derive(Clone, Copy, Debug)]
struct Facts {
    /// Where its address space ends.
    end: u64,
}

/// Which arguments of an information service are the class, the buffer, its length and the
/// returned length, and the classes converted.
struct InfoSpec {
    service: Service,
    class: usize,
    buffer: usize,
    length: usize,
    returned: usize,
    classes: &'static [InfoClass],
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// `SYSTEM_BASIC_INFORMATION`: seven `ULONG`s, then `MinimumUserModeAddress`,
/// `MaximumUserModeAddress`, `ActiveProcessorsAffinityMask` (pointer-sized) and
/// `NumberOfProcessors` (`CCHAR`). The address range is the 32-bit process's own: from
/// 0x10000 to the last byte below its end.
fn system_basic(w: &[u8], n: &mut [u8], f: &Facts) {
    n[..28].copy_from_slice(&w[..28]);
    put32(n, 28, crate::vm::LOWEST_ADDRESS as u32);
    put32(n, 32, (f.end - 1) as u32);
    put32(n, 36, u32_at(w, 48));
    n[40] = w[56];
}

/// `MEMORY_BASIC_INFORMATION` (`winnt.h`): `BaseAddress`, `AllocationBase`, `AllocationProtect`,
/// `RegionSize`, `State`, `Protect`, `Type`.
fn memory_basic(w: &[u8], n: &mut [u8], _: &Facts) {
    put32(n, 0, u32_at(w, 0));
    put32(n, 4, u32_at(w, 8));
    put32(n, 8, u32_at(w, 16));
    put32(n, 12, u32_at(w, 24));
    put32(n, 16, u32_at(w, 32));
    put32(n, 20, u32_at(w, 36));
    put32(n, 24, u32_at(w, 40));
}

/// `PROCESS_BASIC_INFORMATION` (`winternl.h`): `ExitStatus`, `PebBaseAddress`,
/// `AffinityMask`, `BasePriority`, `UniqueProcessId`, `InheritedFromUniqueProcessId`.
fn process_basic(w: &[u8], n: &mut [u8], _: &Facts) {
    for (i, at) in [0usize, 8, 16, 24, 32, 40].into_iter().enumerate() {
        put32(n, 4 * i, u32_at(w, at));
    }
}

/// `THREAD_BASIC_INFORMATION`: `ExitStatus`, `TebBaseAddress`, `ClientId` (two handles),
/// `AffinityMask`, `Priority`, `BasePriority`.
fn thread_basic(w: &[u8], n: &mut [u8], _: &Facts) {
    for (i, at) in [0usize, 8, 16, 24, 32, 40, 44].into_iter().enumerate() {
        put32(n, 4 * i, u32_at(w, at));
    }
}

/// `ProcessWow64Information` (Learn: `NtQueryInformationProcess`): a `ULONG_PTR`.
fn ulong_ptr(w: &[u8], n: &mut [u8], _: &Facts) {
    put32(n, 0, u32_at(w, 0));
}

const INFO: &[InfoSpec] = &[
    InfoSpec {
        service: Service::QuerySystemInformation,
        class: 0,
        buffer: 1,
        length: 2,
        returned: 3,
        classes: &[InfoClass { class: 0, wide: 64, narrow: 44, convert: system_basic }],
    },
    InfoSpec {
        service: Service::QueryVirtualMemory,
        class: 2,
        buffer: 3,
        length: 4,
        returned: 5,
        classes: &[InfoClass { class: 0, wide: 48, narrow: 28, convert: memory_basic }],
    },
    InfoSpec {
        service: Service::QueryInformationProcess,
        class: 1,
        buffer: 2,
        length: 3,
        returned: 4,
        classes: &[
            InfoClass { class: 0, wide: 48, narrow: 24, convert: process_basic },
            InfoClass { class: 26, wide: 8, narrow: 4, convert: ulong_ptr },
        ],
    },
    InfoSpec {
        service: Service::QueryInformationThread,
        class: 1,
        buffer: 2,
        length: 3,
        returned: 4,
        classes: &[InfoClass { class: 0, wide: 48, narrow: 28, convert: thread_basic }],
    },
];

/// A converted information call: where the 64-bit result goes and what the caller gets.
#[derive(Debug)]
struct InfoCall {
    class: &'static InfoClass,
    wide: u64,
    narrow: u64,
    returned: u64,
}

/// A call ready to run, or answered already.
#[allow(clippy::large_enum_variant)]
enum Prepared {
    Done(NtStatus),
    Call { args: Vec<u64>, backs: Vec<Back>, info: Option<InfoCall>, scratch: Scratch },
}

fn read32<K: NtKernel>(k: &mut K, addr: u64) -> Result<u32, NtStatus> {
    abi::read_u32(k, addr)
}

fn sign(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// The 64-bit copy of the 32-bit `UNICODE_STRING` at `at` (`Length`, `MaximumLength`,
/// `Buffer`), in the window.
fn unicode<K: NtKernel>(k: &mut K, s: &mut Scratch, at: u64) -> Result<u64, NtStatus> {
    let mut b = [0u8; 8];
    abi::read(k, at, &mut b)?;
    let wide = s.alloc(16)?;
    let mut w = [0u8; 16];
    w[..4].copy_from_slice(&b[..4]);
    w[8..12].copy_from_slice(&b[4..8]);
    s.put(wide, &w);
    Ok(wide)
}

/// The 64-bit copy of the 32-bit `OBJECT_ATTRIBUTES` at `at` (`ntdef.h`: `Length`,
/// `RootDirectory`, `ObjectName`, `Attributes`, `SecurityDescriptor`,
/// `SecurityQualityOfService`; 24 bytes, 48 in 64 bits), with its name.
fn object_attributes<K: NtKernel>(k: &mut K, s: &mut Scratch, at: u64, file: bool) -> Result<u64, NtStatus> {
    let mut b = [0u8; 24];
    abi::read(k, at, &mut b)?;
    let field = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().expect("4 bytes"));
    let mut name = if field(2) != 0 { unicode(k, s, u64::from(field(2)))? } else { 0 };
    // A file named by a full path under `%windir%\System32` is the one under `SysWOW64`
    // (Learn: "File System Redirector"): the copy gets the redirected name.
    if file && name != 0 && field(1) == 0 {
        // The caller's 32-bit `UNICODE_STRING`: `Length`, then `Buffer` at 4.
        let at = u64::from(field(2));
        let length = usize::from(u16::from_le_bytes(s.get(name, 2).try_into().expect("2 bytes")));
        let mut chars = alloc::vec![0u8; length & !1];
        let buffer = u64::from(read32(k, at + 4)?);
        abi::read(k, buffer, &mut chars)?;
        let path: Vec<u16> = chars.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        if let Some(to) = super::redirect_system32(&path) {
            let bytes: Vec<u8> = to.iter().flat_map(|c| c.to_le_bytes()).collect();
            let length = u16::try_from(bytes.len()).map_err(|_| poly_ntobj::status::STATUS_OBJECT_NAME_INVALID)?;
            let buffer = s.alloc(bytes.len())?;
            s.put(buffer, &bytes);
            name = s.alloc(16)?;
            let mut w = [0u8; 16];
            w[0..2].copy_from_slice(&length.to_le_bytes());
            w[2..4].copy_from_slice(&length.to_le_bytes());
            w[8..16].copy_from_slice(&buffer.to_le_bytes());
            s.put(name, &w);
        }
    }
    let wide = s.alloc(48)?;
    let mut w = [0u8; 48];
    w[0..4].copy_from_slice(&48u32.to_le_bytes());
    w[8..16].copy_from_slice(&sign(field(1)).to_le_bytes());
    w[16..24].copy_from_slice(&name.to_le_bytes());
    w[24..28].copy_from_slice(&field(3).to_le_bytes());
    w[32..40].copy_from_slice(&u64::from(field(4)).to_le_bytes());
    w[40..48].copy_from_slice(&u64::from(field(5)).to_le_bytes());
    s.put(wide, &w);
    Ok(wide)
}

/// The services that name a file by its `OBJECT_ATTRIBUTES`: their names are redirected.
const FILE_SERVICES: &[Service] = &[
    Service::CreateFile,
    Service::OpenFile,
    Service::DeleteFile,
    Service::QueryAttributesFile,
    Service::QueryFullAttributesFile,
];

/// Read and widen the arguments of a call to `service` whose 32-bit arguments start at `at`.
fn prepare<K: NtKernel>(k: &mut K, at: u64, service: Service, sig: &[Arg]) -> Result<Prepared, NtStatus> {
    let mut narrow = Vec::with_capacity(sig.len());
    for i in 0..sig.len() {
        narrow.push(read32(k, at + 4 * i as u64)?);
    }
    let mut s = Scratch::new();
    let mut args = Vec::with_capacity(sig.len());
    let mut backs = Vec::new();
    for (&kind, &v) in sig.iter().zip(&narrow) {
        let p = u64::from(v);
        let wide = match kind {
            Arg::V | Arg::P => p,
            Arg::H => sign(v),
            Arg::X if v == 0 => 0,
            Arg::X => return Ok(Prepared::Done(STATUS_NOT_IMPLEMENTED)),
            _ if v == 0 => 0,
            Arg::OH | Arg::OS => {
                let now = read32(k, p)?;
                let wide = s.alloc(8)?;
                let value = if kind == Arg::OH { sign(now) } else { u64::from(now) };
                s.put(wide, &value.to_le_bytes());
                backs.push(Back::Low32 { narrow: p, wide });
                wide
            }
            Arg::IO => {
                let mut b = [0u8; 8];
                abi::read(k, p, &mut b)?;
                let wide = s.alloc(16)?;
                let mut w = [0u8; 16];
                w[..4].copy_from_slice(&b[..4]);
                w[8..12].copy_from_slice(&b[4..8]);
                s.put(wide, &w);
                backs.push(Back::Iosb { narrow: p, wide });
                wide
            }
            Arg::OA => object_attributes(k, &mut s, p, FILE_SERVICES.contains(&service))?,
            Arg::US => unicode(k, &mut s, p)?,
            Arg::UO => {
                let wide = unicode(k, &mut s, p)?;
                backs.push(Back::Lengths { narrow: p, wide });
                wide
            }
            Arg::CI => {
                let (process, thread) = (read32(k, p)?, read32(k, p + 4)?);
                let wide = s.alloc(16)?;
                s.put(wide, &u64::from(process).to_le_bytes());
                s.put(wide + 8, &u64::from(thread).to_le_bytes());
                wide
            }
            Arg::CX => {
                let mut b = alloc::vec![0u8; super::context::CONTEXT32_SIZE];
                abi::read(k, p, &mut b)?;
                let bytes = super::context::from_context32(&b).to_bytes();
                let wide = s.alloc(bytes.len())?;
                s.put(wide, &bytes);
                wide
            }
            Arg::ER => {
                let mut b = [0u8; super::context::EXCEPTION_RECORD32_SIZE];
                abi::read(k, p, &mut b)?;
                let Some(record) = super::context::from_record32(&b) else {
                    return Ok(Prepared::Done(poly_ntobj::status::STATUS_INVALID_PARAMETER));
                };
                let bytes = record.to_bytes();
                let wide = s.alloc(bytes.len())?;
                s.put(wide, &bytes);
                wide
            }
            Arg::HA => {
                // MAXIMUM_WAIT_OBJECTS (64): the service refuses more before it reads them.
                let count = narrow.first().copied().unwrap_or(0).min(64) as u64;
                let wide = s.alloc(8 * count as usize)?;
                for j in 0..count {
                    let h = sign(read32(k, p + 4 * j)?);
                    s.put(wide + 8 * j, &h.to_le_bytes());
                }
                wide
            }
        };
        args.push(wide);
    }
    // An information class with pointer-sized fields: the service writes the 64-bit form into
    // the window, the caller gets the 32-bit one.
    let mut info = None;
    if let Some(spec) = INFO.iter().find(|s| s.service == service)
        && let Some(class) = spec.classes.iter().find(|c| c.class == narrow[spec.class])
    {
        let (buffer, length, returned) =
            (u64::from(narrow[spec.buffer]), narrow[spec.length] as usize, u64::from(narrow[spec.returned]));
        if buffer == 0 || length < class.narrow {
            if returned != 0 {
                abi::write_u32(k, returned, class.narrow as u32)?;
            }
            return Ok(Prepared::Done(STATUS_INFO_LENGTH_MISMATCH));
        }
        let wide = s.alloc(class.wide)?;
        args[spec.buffer] = wide;
        args[spec.length] = class.wide as u64;
        info = Some(InfoCall { class, wide, narrow: buffer, returned });
    }
    Ok(Prepared::Call { args, backs, info, scratch: s })
}

/// The calling 32-bit thread's view of the kernel during one converted call: the scratch window
/// (see the module documentation) over the caller's memory, everything else the caller's.
pub struct Thunked<'a, K: NtKernel> {
    k: &'a mut K,
    scratch: Scratch,
}

impl<K: NtKernel> core::fmt::Debug for Thunked<'_, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Thunked").field("scratch", &self.scratch.bytes.len()).finish_non_exhaustive()
    }
}

impl<K: NtKernel> Thunked<'_, K> {
    /// The kernel underneath.
    pub fn inner(&mut self) -> &mut K {
        self.k
    }
}

/// Run the 32-bit system call in `regs` (EAX = the number, EDX = the arguments' address) against
/// `k`: widen it, run it with `run` (normally `crate::dispatch_with`, wrapped in whatever the
/// kernel adds, such as its trace), write the results back and put the `NTSTATUS` in `rax`. A
/// service with no 32-bit signature answers `STATUS_NOT_IMPLEMENTED`.
pub fn dispatch32<K: NtKernel>(
    k: &mut K,
    regs: &mut SyscallRegs,
    run: impl FnOnce(&mut Thunked<'_, K>, &mut SyscallRegs) -> Outcome,
) -> Outcome {
    let number = regs.rax as u32;
    let found = Service::from_number(number).and_then(|s| sigs::signature(s).map(|sig| (s, sig)));
    let Some((service, sig)) = found else {
        let (table, index) = crate::decode(number);
        regs.rax = u64::from(crate::STATUS_NOT_IMPLEMENTED);
        return Outcome::Unimplemented { table: table.name(), number: u64::from(index) };
    };
    let end = k.with_state(|_, p, _| p.vm.end());
    let facts = Facts { end };
    let (args, backs, info, scratch) = match prepare(k, regs.rdx & 0xFFFF_FFFF, service, sig) {
        Ok(Prepared::Call { args, backs, info, scratch }) => (args, backs, info, scratch),
        Ok(Prepared::Done(status)) | Err(status) => {
            regs.rax = u64::from(status.0);
            return Outcome::Handled;
        }
    };
    let mut wide = SyscallRegs {
        rax: u64::from(number),
        rdi: 0,
        rsi: 0,
        rdx: args.get(1).copied().unwrap_or(0),
        r10: args.first().copied().unwrap_or(0),
        r8: args.get(2).copied().unwrap_or(0),
        r9: args.get(3).copied().unwrap_or(0),
        rflags: regs.rflags,
        rsp: SCRATCH_BASE,
    };
    let mut t = Thunked { k, scratch };
    for (i, a) in args.iter().enumerate().skip(4) {
        t.scratch.put(SCRATCH_BASE + 0x28 + 8 * (i as u64 - 4), &a.to_le_bytes());
    }
    let outcome = run(&mut t, &mut wide);
    let status = NtStatus(wide.rax as u32);
    let Thunked { k, scratch } = t;
    let written = write_back(k, &scratch, &backs, info.as_ref(), status, &facts);
    regs.rax = match written {
        Ok(()) => u64::from(status.0),
        Err(e) => u64::from(e.0),
    };
    outcome
}

fn write_back<K: NtKernel>(
    k: &mut K,
    s: &Scratch,
    backs: &[Back],
    info: Option<&InfoCall>,
    status: NtStatus,
    facts: &Facts,
) -> Result<(), NtStatus> {
    for b in backs {
        match *b {
            Back::Low32 { narrow, wide } => abi::write_u32(k, narrow, s.u64_at(wide) as u32)?,
            Back::Iosb { narrow, wide } => {
                let mut n = [0u8; 8];
                n[..4].copy_from_slice(&s.get(wide, 4)[..4]);
                n[4..].copy_from_slice(&s.get(wide + 8, 4)[..4]);
                abi::write(k, narrow, &n)?;
            }
            Back::Lengths { narrow, wide } => abi::write(k, narrow, &s.get(wide, 4)[..4])?,
        }
    }
    if let Some(i) = info {
        if status.is_success() {
            let mut n = alloc::vec![0u8; i.class.narrow];
            (i.class.convert)(s.get(i.wide, i.class.wide), &mut n, facts);
            abi::write(k, i.narrow, &n)?;
        }
        // The returned length is the 32-bit size (a `ULONG` or, for `NtQueryVirtualMemory`, a
        // `SIZE_T`, whose low half the generic write-back already wrote).
        if i.returned != 0 {
            abi::write_u32(k, i.returned, i.class.narrow as u32)?;
        }
    }
    Ok(())
}

impl<K: NtKernel> NtKernel for Thunked<'_, K> {
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault> {
        if Scratch::claims(addr) {
            let r = self.scratch.range(addr, buf.len()).ok_or(UserFault)?;
            buf.copy_from_slice(&self.scratch.bytes[r]);
            return Ok(());
        }
        self.k.read_user(addr, buf)
    }

    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault> {
        if Scratch::claims(addr) {
            let r = self.scratch.range(addr, data.len()).ok_or(UserFault)?;
            self.scratch.bytes[r].copy_from_slice(data);
            return Ok(());
        }
        self.k.write_user(addr, data)
    }

    fn caller(&self) -> Caller {
        self.k.caller()
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R {
        self.k.with_state(f)
    }

    fn terminate_process(&mut self, status: NtStatus) {
        self.k.terminate_process(status)
    }

    fn terminate_current_thread(&mut self, status: NtStatus) {
        self.k.terminate_current_thread(status)
    }

    fn create_thread(&mut self, req: ThreadCreate) -> Result<ThreadCreated, NtStatus> {
        self.k.create_thread(req)
    }

    fn create_process(&mut self, req: ProcessCreate) -> Result<ProcessCreated, NtStatus> {
        self.k.create_process(req)
    }

    fn terminate_thread(&mut self, thread: ThreadRef, status: NtStatus) -> Result<(), NtStatus> {
        self.k.terminate_thread(thread, status)
    }

    fn with_process<R>(
        &mut self,
        pid: poly_ntobj::ProcessId,
        f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R,
    ) -> Option<R> {
        self.k.with_process(pid, f)
    }

    fn terminate_other_process(&mut self, pid: poly_ntobj::ProcessId, status: NtStatus) -> Result<(), NtStatus> {
        self.k.terminate_other_process(pid, status)
    }

    fn suspend_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        self.k.suspend_thread(thread)
    }

    fn resume_thread(&mut self, thread: ThreadRef) -> Result<u32, NtStatus> {
        self.k.resume_thread(thread)
    }

    fn suspend_process(&mut self, pid: poly_ntobj::ProcessId) -> Result<(), NtStatus> {
        self.k.suspend_process(pid)
    }

    fn resume_process(&mut self, pid: poly_ntobj::ProcessId) -> Result<(), NtStatus> {
        self.k.resume_process(pid)
    }

    fn get_context(&mut self, thread: ThreadRef) -> Result<NtContext, NtStatus> {
        self.k.get_context(thread)
    }

    fn set_context(&mut self, thread: ThreadRef, ctx: &NtContext, want: u32) -> Result<(), NtStatus> {
        self.k.set_context(thread, ctx, want)
    }

    fn wait(&mut self, id: WaitId, timeout: NtTimeout, alertable: bool) -> NtStatus {
        self.k.wait(id, timeout, alertable)
    }

    fn arm_timer(
        &mut self,
        timer: ObjectId,
        due: NtTimeout,
        period_ms: u32,
        apc: Option<Apc>,
    ) -> Result<bool, NtStatus> {
        self.k.arm_timer(timer, due, period_ms, apc)
    }

    fn disarm_timer(&mut self, timer: ObjectId) -> Result<bool, NtStatus> {
        self.k.disarm_timer(timer)
    }

    fn queue_apc(&mut self, thread: ThreadRef, apc: Apc) -> Result<(), NtStatus> {
        self.k.queue_apc(thread, apc)
    }

    fn test_alert(&mut self) -> NtStatus {
        self.k.test_alert()
    }

    fn delay(&mut self, timeout: NtTimeout, alertable: bool) -> NtStatus {
        self.k.delay(timeout, alertable)
    }

    fn yield_execution(&mut self) -> NtStatus {
        self.k.yield_execution()
    }

    fn resume_apc(&mut self) -> NtStatus {
        self.k.resume_apc()
    }

    fn trace(&mut self, values: [u64; 3]) {
        self.k.trace(values)
    }

    fn debug_print(&mut self, text: &[u8]) {
        self.k.debug_print(text)
    }

    fn system_time(&self) -> u64 {
        self.k.system_time()
    }

    fn monotonic_100ns(&self) -> u64 {
        self.k.monotonic_100ns()
    }

    fn processor_count(&self) -> u32 {
        self.k.processor_count()
    }

    fn process_times(&self, pid: poly_ntobj::ProcessId) -> Option<[u64; 3]> {
        self.k.process_times(pid)
    }

    fn save_hive(&mut self, file: &str, data: &[u8]) -> Result<(), NtStatus> {
        self.k.save_hive(file, data)
    }

    fn socket_create(
        &mut self,
        domain: crate::afd::Domain,
        ty: crate::afd::SockType,
        protocol: crate::afd::Protocol,
    ) -> Result<alloc::sync::Arc<dyn crate::afd::NtSocket>, NtStatus> {
        self.k.socket_create(domain, ty, protocol)
    }

    fn socket_poll(
        &mut self,
        socks: &[(alloc::sync::Arc<dyn crate::afd::NtSocket>, crate::afd::Readiness)],
        wait: crate::afd::SockWait,
    ) -> Result<Vec<crate::afd::Readiness>, NtStatus> {
        self.k.socket_poll(socks, wait)
    }

    fn socket_pend(&mut self, io: crate::afd::PendIo) -> Result<(), crate::afd::PendIo> {
        self.k.socket_pend(io)
    }

    fn net_info(&self) -> (Vec<[u8; 4]>, alloc::string::String) {
        self.k.net_info()
    }

    fn net_interfaces(&self) -> Vec<poly_iphlp::IfInfo> {
        self.k.net_interfaces()
    }

    fn load_context(&mut self, ctx: &poly_seh::Context, flags: u32) -> Result<(), NtStatus> {
        self.k.load_context(ctx, flags)
    }

    fn exception_delivered(&mut self, record: &poly_seh::ExceptionRecord, ctx: &poly_seh::Context) {
        self.k.exception_delivered(record, ctx)
    }

    fn unhandled_exception(&mut self, record: &poly_seh::ExceptionRecord, ctx: &poly_seh::Context) {
        self.k.unhandled_exception(record, ctx)
    }
}
