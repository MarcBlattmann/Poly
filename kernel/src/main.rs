//! Poly kernel: one Rust core hosting the Linux, NT and XNU personalities.

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod arch;
mod block;
mod boot_info;
mod cputime;
mod display;
mod dma;
mod frames;
mod fs;
mod heap;
mod keyboard;
mod ktest;
mod linux;
mod log;
mod mm;
mod net;
mod pci;
mod power;
mod process;
mod program;
mod qemu;
mod random;
mod selftest;
mod shell;
mod sync;
mod syscall;
mod terminal;
mod testrun;
mod thread;
mod time;
mod user;
mod vdso;
mod virtio_blk;

use alloc::boxed::Box;
use core::panic::PanicInfo;
use poly_mem::{MemoryMap, PAGE_SIZE, Region, RegionKind};

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
    // The initramfs's pages stay out of every allocator until it is unpacked.
    let (memory, initrd) = fs::initrd::reserve(&boot, &memory);

    // SAFETY: once, on the boot CPU with interrupts off, before the frame
    // allocator. `boot` copied what it needed out of the PVH start info, so
    // nothing holds a pointer into the boot mapping.
    let memory = unsafe { mm::init(&memory) };
    // SAFETY: once, on the boot CPU with interrupts off, after the kernel
    // page tables and the GDT, before any user thread.
    unsafe { arch::user::init() };

    let (memory, meta) = frames::init(&memory, initrd);
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
    // SAFETY: once, on the boot CPU with interrupts still off, after the
    // heap, the scheduler and the ACPI tables.
    if let Err(e) = unsafe { arch::init_interrupts(acpi::platform().as_ref()) } {
        panic!("interrupt and timer setup failed: {e}");
    }
    // SAFETY: the boot thread holds no lock; from here on the tick preempts it.
    unsafe { arch::cpu::enable_interrupts() };
    kprintln!("irq: interrupts enabled");
    pci::init();
    block::init();
    keyboard::init(&boot);
    display::init(&boot);
    fs::init(&boot, initrd);
    net::init(&boot);
    // `poly.init=<path>`: the first program runs before anything else; a
    // test run (`poly.qemu_exit`) ends with it (ADR 0027, ADR 0038).
    program::run_init(&boot);

    // The shell's `test` command runs the tests again later.
    let memory: &'static MemoryMap = Box::leak(Box::new(memory));
    shell::set_test_env(&boot, memory);
    let ok = selftest::run(&boot, memory);
    heap::log_stats();
    selftest::trigger_requested_fault(&boot);
    // An automated test run ends here, unless it asked for the shell (and
    // the tests passed): then a boot test types into it.
    if boot.has_flag("poly.qemu_exit") && !(ok && boot.has_flag(shell::SHELL_FLAG)) {
        qemu::exit(if ok { qemu::ExitCode::Success } else { qemu::ExitCode::Failure });
    }
    // The screen terminal (if there is a display) logs as it starts, so
    // before the serial shell's first prompt.
    terminal::start(&boot);
    // Log before the shell starts: once its thread runs, the prompt may come
    // first and a later log line would land after it.
    kprintln!("boot: done; starting the shell on COM1 (type 'help')");
    if let Err(e) = shell::start_serial() {
        kprintln!("boot: the shell could not start: {e:?}");
    }
    // The idle thread (and the shell) take over, halting between interrupts.
    thread::exit(0)
}

fn kernel_phys_start() -> u64 {
    &raw const __kernel_phys_start as u64
}

fn kernel_phys_end() -> u64 {
    &raw const __kernel_phys_end as u64
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // No preemption or interrupt handlers while the report is printed.
    arch::cpu::disable_interrupts();
    log::emergency_print(format_args!("\nKERNEL PANIC: {info}\n"));
    arch::backtrace::print_for_panic();
    qemu::exit(qemu::ExitCode::Failure);
    arch::cpu::halt_forever()
}
