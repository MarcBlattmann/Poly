//! The entry point, in assembly, and where the boot information comes from.
//!
//! QEMU's `virt` machine with `-kernel <ELF>` loads the ELF's segments at
//! their addresses and starts the boot CPU at the ELF entry point, in EL1
//! (or in EL2 with `-machine virt,virtualization=on`), MMU and caches off,
//! with `x0` holding the address of the devicetree blob. The other CPUs
//! (`-smp`) are parked by QEMU's own boot code until PSCI starts them, but the
//! stub parks any CPU that is not CPU 0 itself, so it also works where all
//! CPUs start together. QEMU documents the blob as being "at the start of RAM"
//! for bare-metal guests; [`find_dtb`] tries `x0` first, then there.
//!
//! The assembly: park CPUs other than 0, remember the entry exception level,
//! leave EL2 for EL1 if entered there, enable FP/SIMD (compiled code uses it),
//! set the stack and the vector table, clear `.bss`, call [`crate::a64_main`].

use core::arch::global_asm;
use poly_aarch64_stub::fdt::{self, Fdt};

global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
_start:
    // Only CPU 0 (all affinity fields 0) boots; the others wait forever.
    mrs     x9, mpidr_el1
    and     x9, x9, #0xffffff
    cbnz    x9, a64_park
    mov     x19, x0                 // the devicetree, kept for Rust
    mrs     x20, CurrentEL
    lsr     x20, x20, #2            // the exception level entered at
    cmp     x20, #2
    b.eq    a64_leave_el2
    cmp     x20, #1
    b.ne    a64_park                // EL3 and EL0 are not supported

a64_el1:
    // CPACR_EL1.FPEN = 0b11: no trap on FP and SIMD at EL1 and EL0.
    mov     x1, #(3 << 20)
    msr     cpacr_el1, x1
    isb
    adrp    x1, __stack_top
    add     x1, x1, :lo12:__stack_top
    mov     sp, x1
    adrp    x1, a64_vectors
    add     x1, x1, :lo12:a64_vectors
    msr     vbar_el1, x1
    isb
    // Clear .bss, 16 bytes at a time (the linker script aligns both ends).
    adrp    x1, __bss_start
    add     x1, x1, :lo12:__bss_start
    adrp    x2, __bss_end
    add     x2, x2, :lo12:__bss_end
1:  cmp     x1, x2
    b.hs    2f
    stp     xzr, xzr, [x1], #16
    b       1b
2:  mov     x0, x19
    mov     x1, x20
    bl      a64_main
a64_park:
    wfe
    b       a64_park

a64_leave_el2:
    // EL1 runs AArch64 (HCR_EL2.RW); every other trap to EL2 is off.
    mov     x1, #0x80000000
    msr     hcr_el2, x1
    // EL1 may use the counter and the physical timer (CNTHCTL_EL2.EL1PCTEN
    // and EL1PCEN), with no offset.
    mrs     x1, cnthctl_el2
    orr     x1, x1, #3
    msr     cnthctl_el2, x1
    msr     cntvoff_el2, xzr
    // No FP/SIMD or trace traps to EL2: CPTR_EL2's RES1 bits only.
    mov     x1, #0x33ff
    msr     cptr_el2, x1
    // EL1 reads the real ID registers.
    mrs     x1, midr_el1
    msr     vpidr_el2, x1
    mrs     x1, mpidr_el1
    msr     vmpidr_el2, x1
    // SCTLR_EL1: reset value with the MMU, caches and alignment checks off
    // (the RES1 bits: 29, 28, 23, 22, 20, 11).
    mov     x1, #0x0800
    movk    x1, #0x30d0, lsl #16
    msr     sctlr_el1, x1
    // Return to EL1h (its own stack pointer) with D, A, I and F masked.
    mov     x1, #0x3c5
    msr     spsr_el2, x1
    adr     x1, a64_el1
    msr     elr_el2, x1
    eret
"#
);

unsafe extern "C" {
    static __image_start: u8;
    static __image_end: u8;
}

/// The address range the stub's image occupies, stack included.
pub fn image() -> (u64, u64) {
    // Only the addresses of the linker script's symbols are taken, never
    // their contents.
    (&raw const __image_start as u64, &raw const __image_end as u64)
}

/// QEMU's `virt` RAM base: where the blob is when `x0` is no use (QEMU
/// documentation, "Arm Virt": "RAM starts at 0x4000_0000").
const RAM_START: usize = 0x4000_0000;
/// The largest blob accepted.
const MAX_DTB: usize = 2 << 20;

/// The devicetree blob: at `x0`, else at the start of RAM, whichever has the
/// magic number. Returns its address and size.
///
/// # Safety
/// The MMU must be off or map all of RAM, and `x0` is either a readable
/// address or garbage the caller accepts reading 8 bytes at (the machine put
/// it there).
pub unsafe fn find_dtb(x0: usize) -> Option<(usize, usize)> {
    for addr in [x0, RAM_START] {
        if addr == 0 || !addr.is_multiple_of(8) {
            continue;
        }
        // SAFETY: caller's contract: the blob's header is readable.
        let header = unsafe { core::slice::from_raw_parts(addr as *const u8, 8) };
        if let Ok(size) = fdt::total_size(header)
            && (40..=MAX_DTB).contains(&size)
        {
            return Some((addr, size));
        }
    }
    None
}

/// What the devicetree says, with the fallbacks used where it says nothing.
#[derive(Debug)]
pub struct BootInfo {
    pub dtb: Option<(usize, usize)>,
    /// RAM: base and size, `None` if the blob has no memory node.
    pub ram: Option<(u64, u64)>,
    pub uart: Option<u64>,
    pub bootargs: Option<&'static str>,
    pub psci: Option<poly_aarch64_stub::psci::Conduit>,
}

impl BootInfo {
    /// Read the blob at `dtb`.
    ///
    /// # Safety
    /// `dtb` was returned by [`find_dtb`], and the blob stays in place and
    /// unchanged for the rest of the run (it is below the image and nothing
    /// allocates there before the frame allocator reserves it).
    pub unsafe fn read(dtb: Option<(usize, usize)>) -> BootInfo {
        let mut info = BootInfo { dtb, ram: None, uart: None, bootargs: None, psci: None };
        let Some((addr, size)) = dtb else { return info };
        // SAFETY: caller's contract: `size` bytes at `addr` are the blob.
        let bytes: &'static [u8] = unsafe { core::slice::from_raw_parts(addr as *const u8, size) };
        let Ok(fdt) = Fdt::new(bytes) else { return info };
        info.ram = fdt.memory().map(|r| (r.base, r.size));
        info.uart = fdt.find_compatible("arm,pl011").and_then(|n| n.reg()).map(|r| r.base);
        info.bootargs = fdt.bootargs();
        info.psci = fdt.psci_method().and_then(poly_aarch64_stub::psci::Conduit::parse);
        info
    }
}
