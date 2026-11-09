//! The aarch64 boot stub (ADR 0154): boots on QEMU's `virt` machine, prints to
//! its PL011, runs a small self-test and ends QEMU with a status through Arm
//! semihosting. Everything specific to the architecture lives here and in the
//! sibling modules; the logic that touches no hardware is `poly_aarch64_stub` (lib.rs).

#![no_std]
#![no_main]

extern crate alloc;

mod boot;
mod cpu;
mod exceptions;
mod heap;
mod paging;
mod selftest;
mod semihosting;
mod uart;

use boot::BootInfo;
use core::fmt::Write as _;
use poly_aarch64_stub::cmdline::has_flag;
use poly_aarch64_stub::regs::{self, Midr};
use poly_aarch64_stub::semihost::status;
use poly_mem::{MemoryMap, Region, RegionKind};

/// Named on the command line (`-append`) to make the boot end in a panic, to
/// see the panic path work (like the x86 kernel's `poly.selftest.oom`).
const PANIC_FLAG: &str = "poly.selftest.panic";

/// RAM when the devicetree has no memory node: QEMU `virt`'s 128 MiB at
/// 0x4000_0000 (QEMU documentation, "Arm Virt").
const FALLBACK_RAM: (u64, u64) = (0x4000_0000, 128 << 20);

/// Called from the assembly in `boot.rs` on the boot CPU, with the stack and
/// `.bss` set up: `dtb_addr` is `x0` as the machine left it, `entry_el` the
/// exception level the CPU started in.
#[unsafe(no_mangle)]
extern "C" fn a64_main(dtb_addr: usize, entry_el: u64) -> ! {
    // SAFETY: the MMU is still off, so all of physical memory is readable;
    // `x0` is what QEMU passed and the start of RAM is its documented
    // fallback.
    let dtb = unsafe { boot::find_dtb(dtb_addr) };
    // SAFETY: `find_dtb` validated the header, the blob lies below the
    // image, and nothing writes it.
    let info = unsafe { BootInfo::read(dtb) };
    uart::init(info.uart.map_or(uart::QEMU_VIRT_PL011, |u| u as usize));
    if let Some(conduit) = info.psci {
        semihosting::set_conduit(conduit);
    }

    banner(&info, entry_el);
    let ram = info.ram.unwrap_or(FALLBACK_RAM);
    let (image_start, image_end) = boot::image();

    // The frame allocator manages RAM except the image and the blob.
    let mut map = MemoryMap::new();
    let mut push = |start: u64, end: u64, kind| map.push(Region::new(start, end, kind)).is_ok();
    let mut ok = push(ram.0, ram.0 + ram.1, RegionKind::Usable);
    if let Some((addr, size)) = info.dtb {
        ok &= push(addr as u64, (addr + size) as u64, RegionKind::Reserved);
    }
    ok &= push(image_start, image_end, RegionKind::Kernel);
    let usable = ok.then(|| map.normalized().ok()).flatten();
    match usable.as_ref().map(heap::init) {
        Some(Ok(())) => {}
        other => panic!("the frame allocator did not start: {other:?}"),
    }

    let tally = selftest::run(&info, ram, entry_el);
    let _ = tally.summary(&mut uart::Console);

    if info.bootargs.is_some_and(|args| has_flag(args, PANIC_FLAG)) {
        panic!("{PANIC_FLAG}: a deliberate panic to test the panic path");
    }
    semihosting::exit(if tally.all_passed() { status::PASS } else { status::FAIL })
}

fn banner(info: &BootInfo, entry_el: u64) {
    let mut out = uart::Console;
    let _ = writeln!(out, "Poly aarch64 boot stub");
    let el = regs::el_from_current_el(cpu::current_el());
    let _ = writeln!(out, "CurrentEL: EL{el} (entered at EL{entry_el})");
    let midr = cpu::midr_el1();
    let m = Midr::decode(midr);
    let _ = writeln!(
        out,
        "MIDR_EL1: {midr:#010x} ({} {}, r{}p{})",
        m.implementer_name(),
        m.part_name(),
        m.variant,
        m.revision
    );
    let _ = writeln!(out, "MPIDR_EL1: {:#x}", cpu::mpidr_el1());
    let _ = writeln!(out, "timer frequency: {} Hz", cpu::cntfrq_el0());
    let (image_start, image_end) = boot::image();
    let _ = writeln!(out, "image: {image_start:#x}..{image_end:#x}");
    match info.dtb {
        Some((addr, size)) => {
            let _ = writeln!(out, "devicetree: {addr:#x}, {size} bytes");
        }
        None => {
            let _ = writeln!(out, "devicetree: none found");
        }
    }
    let (base, size) = info.ram.unwrap_or(FALLBACK_RAM);
    let from = if info.ram.is_some() { "devicetree" } else { "fallback" };
    let _ = writeln!(out, "RAM: {base:#x}..{:#x} ({from})", base + size);
    let _ =
        writeln!(out, "PL011: {:#x} ({})", uart::base(), if info.uart.is_some() { "devicetree" } else { "fallback" });
    let _ = writeln!(out, "bootargs: {:?}", info.bootargs.unwrap_or(""));
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let mut out = uart::Console;
    let _ = writeln!(out, "KERNEL PANIC: {info}");
    semihosting::exit(status::PANIC)
}
