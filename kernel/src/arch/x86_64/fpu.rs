//! x87 / SSE (and AVX) register state of user threads.
//!
//! The kernel itself never touches these registers: its target
//! (x86_64-unknown-none) is soft-float, so the compiler emits no x87, SSE
//! or AVX instructions. User code does, so each user thread has a save area
//! ([`FpuArea`]) and the registers are switched with the threads.
//!
//! **Policy: eager, with an owner.** The registers always hold the state of
//! one user thread, the *owner* ([`OWNER`]). Switching to a user thread
//! that is not the owner saves the registers into the owner's area and
//! loads the new thread's; switching to a kernel thread, or back to the
//! owner, costs nothing. CR0.TS stays clear, so there is no lazy #NM trap
//! (nor its information leaks); the kernel has nothing of its own to save
//! because it never uses the registers. A dying owner's area is simply
//! forgotten (its `Drop` clears [`OWNER`]).
//!
//! **Format.** With XSAVE (CPUID.01H:ECX.XSAVE), CR4.OSXSAVE is set and
//! XCR0 enables x87, SSE and, if present, AVX; the area is as large as
//! CPUID.(EAX=0DH,ECX=0):EBX says for those, and `XSAVE64` / `XRSTOR64`
//! move it (SDM Vol. 1 §13). Without XSAVE it is the 512-byte FXSAVE
//! image (`FXSAVE64` / `FXRSTOR64`, SDM Vol. 1 §10.5). Either way it is
//! 64-byte aligned, on the heap.
//!
//! **Initial state.** A new area restores to the state `FNINIT` and a
//! reset MXCSR give: x87 control word 0x37F (all exceptions masked, 64-bit
//! precision, round to nearest), empty x87 stack, MXCSR 0x1F80 (all SIMD
//! exceptions masked), every XMM/YMM register zero. In an XSAVE area the
//! header's XSTATE_BV is 0, which makes XRSTOR load each component's
//! initial configuration (SDM Vol. 1 §13.8.1); MXCSR still comes from the
//! legacy region.

use super::cpu;
use alloc::alloc::{Layout, alloc_zeroed, dealloc};
use core::arch::asm;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// XCR0 bits (SDM Vol. 1 §13.3): x87, SSE (XMM registers and MXCSR), AVX
/// (upper halves of the YMM registers).
const XCR0_X87: u64 = 1 << 0;
const XCR0_SSE: u64 = 1 << 1;
const XCR0_AVX: u64 = 1 << 2;

/// Size of the FXSAVE image and of the XSAVE legacy region.
const LEGACY_SIZE: usize = 512;
/// Offsets in the legacy region (SDM Vol. 1, Table 10-2).
const FCW_OFFSET: usize = 0;
const MXCSR_OFFSET: usize = 24;
/// x87 control word after FNINIT (SDM Vol. 1 §8.1.5).
const FCW_INIT: u16 = 0x37f;
/// MXCSR after reset (SDM Vol. 1 §10.2.3).
const MXCSR_INIT: u32 = 0x1f80;
/// XSAVE requires a 64-byte aligned area; FXSAVE 16.
const ALIGN: usize = 64;

/// The XSAVE header follows the legacy region; XSTATE_BV is its first
/// field, XCOMP_BV the second (SDM Vol. 1 §13.4.2).
const HEADER_OFFSET: usize = LEGACY_SIZE;
const HEADER_SIZE: usize = 64;
/// MXCSR_MASK in an FXSAVE image (SDM Vol. 1 §10.5.1.2); 0 there means
/// the default mask 0xFFBF.
const MXCSR_MASK_OFFSET: usize = 28;
const MXCSR_MASK_DEFAULT: u32 = 0xffbf;

static XSAVE: AtomicBool = AtomicBool::new(false);
static XCR0: AtomicU64 = AtomicU64::new(0);
static AREA_SIZE: AtomicUsize = AtomicUsize::new(LEGACY_SIZE);
/// The MXCSR bits this CPU accepts: setting any other one makes FXRSTOR
/// and XRSTOR fault.
static MXCSR_MASK: AtomicU32 = AtomicU32::new(MXCSR_MASK_DEFAULT);

/// The area whose thread's state is in the registers (null: none).
static OWNER: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());

/// How user state is saved, for the boot log.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    pub xsave: bool,
    pub xcr0: u64,
    pub area_size: usize,
}

pub fn info() -> Info {
    Info {
        xsave: XSAVE.load(Ordering::Relaxed),
        xcr0: XCR0.load(Ordering::Relaxed),
        area_size: AREA_SIZE.load(Ordering::Relaxed),
    }
}

/// Make x87 / SSE (and AVX) usable in user mode and pick the save format.
///
/// # Safety
/// Call once on the boot CPU, before any user thread exists.
pub(super) unsafe fn init() {
    // x87 present and not emulated, no #NM on use, native error reporting.
    let cr0 = (cpu::read_cr0() | cpu::CR0_MP | cpu::CR0_NE) & !(cpu::CR0_EM | cpu::CR0_TS);
    // SAFETY: only the FPU-related bits change; paging and protection stay.
    unsafe { cpu::write_cr0(cr0) };
    let mut cr4 = cpu::read_cr4() | cpu::CR4_OSFXSR | cpu::CR4_OSXMMEXCPT;
    // CPUID.01H:ECX.XSAVE [bit 26].
    let xsave = __cpuid(1).ecx & (1 << 26) != 0 && __cpuid(0).eax >= 0xd;
    if xsave {
        cr4 |= cpu::CR4_OSXSAVE;
    }
    // SAFETY: every x86-64 CPU has FXSR and SSE; OSXSAVE only with XSAVE.
    unsafe { cpu::write_cr4(cr4) };
    if xsave {
        // CPUID.(EAX=0DH,ECX=0):EAX lists the XCR0 bits the CPU supports.
        let supported = u64::from(__cpuid_count(0xd, 0).eax);
        let xcr0 = supported & (XCR0_X87 | XCR0_SSE | XCR0_AVX);
        // SAFETY: OSXSAVE is set and only supported components are enabled
        // (x87 and SSE always are; AVX needs SSE, which is on).
        unsafe { cpu::xsetbv(0, xcr0) };
        // EBX: the area size for the components enabled in XCR0 now.
        let size = __cpuid_count(0xd, 0).ebx as usize;
        AREA_SIZE.store(size.max(LEGACY_SIZE + 64), Ordering::Relaxed);
        XCR0.store(xcr0, Ordering::Relaxed);
    }
    XSAVE.store(xsave, Ordering::Relaxed);
    // MXCSR_MASK as FXSAVE reports it (SDM Vol. 1 §11.6.6).
    #[repr(C, align(16))]
    struct Image([u8; LEGACY_SIZE]);
    let mut image = Image([0; LEGACY_SIZE]);
    // SAFETY: a 512-byte, 16-byte aligned buffer on this stack; the
    // registers are nobody's yet (no user thread exists).
    unsafe { asm!("fxsave64 [{}]", in(reg) image.0.as_mut_ptr(), options(nostack)) };
    let mask = u32::from_le_bytes(image.0[MXCSR_MASK_OFFSET..MXCSR_MASK_OFFSET + 4].try_into().expect("4 bytes"));
    MXCSR_MASK.store(if mask == 0 { MXCSR_MASK_DEFAULT } else { mask }, Ordering::Relaxed);
}

/// Bytes in a thread's FPU image ([`FpuArea::snapshot`]).
pub fn area_size() -> usize {
    AREA_SIZE.load(Ordering::Relaxed)
}

/// The XSAVE feature bits images are saved with (XCR0), or `None` when
/// images are FXSAVE images.
pub fn xfeatures() -> Option<u64> {
    XSAVE.load(Ordering::Relaxed).then(|| XCR0.load(Ordering::Relaxed))
}

/// Why [`FpuArea::load`] refused an image: loading it would fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadImage;

/// One user thread's saved x87 / SSE / AVX state.
#[derive(Debug)]
pub struct FpuArea {
    ptr: NonNull<u8>,
}

// SAFETY: the area is plain memory owned by this value; the only other
// access is the save into it when it is the owner, made with interrupts off
// on the one CPU by the scheduler, which also owns the thread this belongs to.
unsafe impl Send for FpuArea {}

impl FpuArea {
    /// A new area holding the initial state (see the module docs). `None`
    /// if the heap is out of memory.
    pub fn new() -> Option<FpuArea> {
        let layout = Self::layout();
        // SAFETY: the layout is not zero-sized.
        let ptr = NonNull::new(unsafe { alloc_zeroed(layout) })?;
        // SAFETY: the area is at least 512 bytes and 64-byte aligned, so
        // both fields are in bounds and aligned (FCW at 0, MXCSR at 24).
        unsafe {
            ptr.add(FCW_OFFSET).cast::<u16>().write(FCW_INIT);
            ptr.add(MXCSR_OFFSET).cast::<u32>().write(MXCSR_INIT);
        }
        Some(FpuArea { ptr })
    }

    fn layout() -> Layout {
        Layout::from_size_align(AREA_SIZE.load(Ordering::Relaxed), ALIGN).expect("FPU area layout")
    }

    /// Put this area's state into the registers, saving the previous
    /// owner's first; nothing to do if it is there already. Call with
    /// interrupts off, when switching to this area's thread.
    pub fn make_current(&self) {
        let owner = OWNER.load(Ordering::Relaxed);
        if owner == self.ptr.as_ptr() {
            return;
        }
        if let Some(owner) = NonNull::new(owner) {
            // SAFETY: OWNER points at a live area (`Drop` clears it before
            // the memory goes), and the registers hold its thread's state.
            unsafe { save(owner) };
        }
        // SAFETY: `self` is a live area holding a valid state: the initial
        // one or one `save` wrote.
        unsafe { restore(self.ptr) };
        OWNER.store(self.ptr.as_ptr(), Ordering::Relaxed);
    }

    fn is_owner(&self) -> bool {
        OWNER.load(Ordering::Relaxed) == self.ptr.as_ptr()
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the area is `area_size()` bytes, owned by `self`, and the
        // `&mut self` excludes other access from this side; the CPU only
        // writes it in `save`, which callers order (interrupts off).
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), area_size()) }
    }

    /// Copy this thread's state into `out` ([`area_size`] bytes; a shorter
    /// buffer gets a prefix): the registers' current contents if it is the
    /// owner. Call with interrupts off, so it cannot change hands meanwhile.
    pub fn snapshot(&mut self, out: &mut [u8]) {
        debug_assert!(!cpu::interrupts_enabled());
        if self.is_owner() {
            // SAFETY: a live area; the registers hold its thread's state.
            unsafe { save(self.ptr) };
        }
        let bytes = self.bytes_mut();
        let n = out.len().min(bytes.len());
        out[..n].copy_from_slice(&bytes[..n]);
    }

    /// Make `image` this thread's state (and the registers' if it is the
    /// owner): a 512-byte FXSAVE image, or with XSAVE an XSAVE image in the
    /// standard format (legacy region, header, extended components as
    /// laid out for XCR0). Refused, changing nothing, if loading it would
    /// fault: reserved MXCSR bits, a header with XCOMP_BV or reserved bytes
    /// set, components outside XCR0, or a component the image is too short
    /// to hold (SDM Vol. 1 §13.8, Vol. 2B `XRSTOR` and `FXRSTOR`). Call
    /// with interrupts off.
    pub fn load(&mut self, image: &[u8]) -> Result<(), BadImage> {
        debug_assert!(!cpu::interrupts_enabled());
        if image.len() < LEGACY_SIZE {
            return Err(BadImage);
        }
        let mxcsr = u32::from_le_bytes(image[MXCSR_OFFSET..MXCSR_OFFSET + 4].try_into().expect("4 bytes"));
        if mxcsr & !MXCSR_MASK.load(Ordering::Relaxed) != 0 {
            return Err(BadImage);
        }
        let xsave = XSAVE.load(Ordering::Relaxed);
        let size = area_size();
        let xstate_bv = if !xsave {
            0
        } else if image.len() == LEGACY_SIZE {
            // A plain FXSAVE image: x87 and SSE state, the rest initial.
            XCR0_X87 | XCR0_SSE
        } else {
            if image.len() < HEADER_OFFSET + HEADER_SIZE {
                return Err(BadImage);
            }
            let header = &image[HEADER_OFFSET..HEADER_OFFSET + HEADER_SIZE];
            let bv = u64::from_le_bytes(header[..8].try_into().expect("8 bytes"));
            if header[8..].iter().any(|&b| b != 0) || bv & !XCR0.load(Ordering::Relaxed) != 0 {
                return Err(BadImage);
            }
            // Components beyond SSE live past the header, up to the area's
            // size for XCR0.
            if bv & !(XCR0_X87 | XCR0_SSE) != 0 && image.len() < size {
                return Err(BadImage);
            }
            bv
        };
        let owner = self.is_owner();
        let area = self.bytes_mut();
        area.fill(0);
        area[..LEGACY_SIZE].copy_from_slice(&image[..LEGACY_SIZE]);
        if xsave {
            area[HEADER_OFFSET..HEADER_OFFSET + 8].copy_from_slice(&xstate_bv.to_le_bytes());
            let ext = HEADER_OFFSET + HEADER_SIZE;
            if image.len() > ext {
                let n = image.len().min(size) - ext;
                area[ext..ext + n].copy_from_slice(&image[ext..ext + n]);
            }
        }
        if owner {
            // SAFETY: a live area holding a state checked above to load
            // without a fault.
            unsafe { restore(self.ptr) };
        }
        Ok(())
    }

    /// Give this thread the initial state (see the module docs), in the
    /// registers too if it is the owner. Call with interrupts off.
    pub fn reset(&mut self) {
        debug_assert!(!cpu::interrupts_enabled());
        let owner = self.is_owner();
        let area = self.bytes_mut();
        area.fill(0);
        area[FCW_OFFSET..FCW_OFFSET + 2].copy_from_slice(&FCW_INIT.to_le_bytes());
        area[MXCSR_OFFSET..MXCSR_OFFSET + 4].copy_from_slice(&MXCSR_INIT.to_le_bytes());
        if owner {
            // SAFETY: a live area holding the initial state.
            unsafe { restore(self.ptr) };
        }
    }
}

impl Drop for FpuArea {
    fn drop(&mut self) {
        // The registers' contents belong to nobody now; nothing saves them.
        let _ = OWNER.compare_exchange(self.ptr.as_ptr(), core::ptr::null_mut(), Ordering::Relaxed, Ordering::Relaxed);
        // SAFETY: allocated in `new` with this layout (the size never
        // changes after `init`).
        unsafe { dealloc(self.ptr.as_ptr(), Self::layout()) };
    }
}

/// Save the registers into `area`.
///
/// # Safety
/// `area` is a live, 64-byte aligned area of `AREA_SIZE` bytes.
unsafe fn save(area: NonNull<u8>) {
    if XSAVE.load(Ordering::Relaxed) {
        // SAFETY: forwarded from the caller; EDX:EAX = all ones saves every
        // component enabled in XCR0.
        unsafe { asm!("xsave64 [{}]", in(reg) area.as_ptr(), in("eax") u32::MAX, in("edx") u32::MAX, options(nostack)) }
    } else {
        // SAFETY: forwarded from the caller (512 bytes, 16-byte aligned).
        unsafe { asm!("fxsave64 [{}]", in(reg) area.as_ptr(), options(nostack)) }
    }
}

/// Load the registers from `area`.
///
/// # Safety
/// As for [`save`], and the area holds a valid state (reserved MXCSR bits
/// clear, a valid XSAVE header), as `new` and `save` leave it.
unsafe fn restore(area: NonNull<u8>) {
    if XSAVE.load(Ordering::Relaxed) {
        // SAFETY: forwarded from the caller.
        unsafe {
            asm!("xrstor64 [{}]", in(reg) area.as_ptr(), in("eax") u32::MAX, in("edx") u32::MAX, options(nostack, readonly))
        }
    } else {
        // SAFETY: forwarded from the caller.
        unsafe { asm!("fxrstor64 [{}]", in(reg) area.as_ptr(), options(nostack, readonly)) }
    }
}
