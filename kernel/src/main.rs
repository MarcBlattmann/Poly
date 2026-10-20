//! Poly kernel: one Rust core hosting the Linux, NT and XNU personalities.

#![no_std]
#![no_main]

mod acpi;
mod arch;
mod boot_info;
mod frames;
mod ktest;
mod log;
mod process;
mod qemu;
mod selftest;
mod sync;
mod syscall;
mod thread;

use core::panic::PanicInfo;
use poly_mem::{PAGE_SIZE, Region, RegionKind};

unsafe extern "C" {
    static __kernel_phys_start: u8;
    static __kernel_phys_end: u8;
}

/// Called from the boot stub with the PVH start info's physical address.
#[unsafe(no_mangle)]
extern "C" fn kernel_entry(start_info: u64) -> ! {
    log::init();
    kprintln!();
    kprintln!("Poly kernel {} (x86_64)", env!("CARGO_PKG_VERSION"));

    // SAFETY: first thing on the boot CPU, interrupts are off.
    unsafe { arch::init() };
    kprintln!("cpu: gdt, tss and idt loaded");

    // SAFETY: `start_info` is what the PVH loader passed us.
    let boot = match unsafe { boot_info::from_pvh(start_info) } {
        Ok(b) => b,
        Err(e) => panic!("bad boot info: {e:?}"),
    };
    if boot.has_flag("poly.qemu_exit") {
        qemu::enable();
    }
    kprintln!("boot: cmdline \"{}\"", boot.cmdline());
    if let Some(rsdp) = boot.rsdp {
        kprintln!("boot: ACPI RSDP at {rsdp:#x}");
    }

    let mut raw = boot.memory.clone();
    // Real-mode area, BIOS data and the PVH structures live below 1 MiB.
    let reserve = [
        Region::new(0, 0x10_0000, RegionKind::Reserved),
        Region::new(kernel_phys_start(), kernel_phys_end(), RegionKind::Kernel),
    ];
    for r in reserve {
        raw.push(r).expect("memory map full");
    }
    let memory = raw.normalized().expect("memory map full");
    for r in memory.regions() {
        kprintln!("mem: {:#012x}-{:#012x} {:?}", r.start, r.end, r.kind);
    }
    kprintln!(
        "mem: {} MiB usable, kernel image {} KiB",
        memory.total(RegionKind::Usable) >> 20,
        memory.total(RegionKind::Kernel) >> 10
    );

    let (memory, meta) = frames::init(&memory);
    if let Some(f) = frames::FRAMES.lock().as_ref() {
        kprintln!(
            "frames: buddy allocator, {} frames total ({} MiB), {} free, metadata {} KiB at {:#x}",
            f.total_frames(),
            (f.total_frames() * PAGE_SIZE) >> 20,
            f.free_frames(),
            meta.bytes >> 10,
            meta.phys
        );
    }
    thread::init();
    acpi::init(boot.rsdp, &memory);

    let ok = selftest::run(&boot, &memory);
    if boot.has_flag("poly.selftest.page_fault") {
        selftest::trigger_page_fault();
    }
    qemu::exit(if ok { qemu::ExitCode::Success } else { qemu::ExitCode::Failure });

    kprintln!("idle");
    arch::cpu::halt_forever()
}

fn kernel_phys_start() -> u64 {
    &raw const __kernel_phys_start as u64
}

fn kernel_phys_end() -> u64 {
    &raw const __kernel_phys_end as u64
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log::emergency_print(format_args!("\nKERNEL PANIC: {info}\n"));
    arch::backtrace::print_for_panic();
    qemu::exit(qemu::ExitCode::Failure);
    arch::cpu::halt_forever()
}
