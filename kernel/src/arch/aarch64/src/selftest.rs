//! The boot self-test: each check prints `ok <name>` or `not ok <name>`, then a
//! verdict line (`poly_a64::report`). `cargo xtask aarch64 test` lists the
//! names it requires (`tools/xtask/src/aarch64.rs`); keep the two in step.

use crate::boot::{self, BootInfo};
use crate::{cpu, exceptions, heap, paging, uart};
use crate::{kprint, kprintln};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::asm;
use poly_a64::mmu as a64_mmu;
use poly_a64::regs::{self, ec};
use poly_a64::report::Tally;

/// Where the checks put their results.
struct Run {
    tally: Tally,
}

impl Run {
    fn check(&mut self, name: &str, ok: bool) {
        self.check_with(name, ok, format_args!(""));
    }

    fn check_with(&mut self, name: &str, ok: bool, detail: core::fmt::Arguments<'_>) {
        let _ = self.tally.check(&mut uart::Console, name, ok, detail);
    }
}

/// Run every check. `ram` is the RAM the allocator and the page tables use.
pub fn run(info: &BootInfo, ram: (u64, u64), entry_el: u64) -> Tally {
    let mut t = Run { tally: Tally::default() };
    let (image_start, image_end) = boot::image();

    // Before the MMU: the machine as the loader left it.
    t.check_with("entry-el", entry_el == 1 || entry_el == 2, format_args!("entered at EL{entry_el}"));
    let el = regs::el_from_current_el(cpu::current_el());
    t.check_with("running-at-el1", el == 1, format_args!("CurrentEL is EL{el}"));
    t.check("vectors-installed", exceptions::installed());
    let product = core::hint::black_box(3.0f64) * core::hint::black_box(1.5f64);
    t.check("fp-simd", product == 4.5);
    t.check("dtb-found", info.dtb.is_some());
    t.check("dtb-memory", info.ram.is_some_and(|(base, size)| base <= image_start && image_end <= base + size));
    t.check("dtb-uart", info.uart.is_some_and(|u| u as usize == uart::base()));
    t.check("dtb-clear-of-image", info.dtb.is_some_and(|(addr, size)| (addr + size) as u64 <= image_start));

    exceptions::expect(ec::BRK_AARCH64);
    // SAFETY: `brk` raises a synchronous exception; the vector table
    // (installed above) records it and returns past this instruction.
    unsafe { asm!("brk #0x42") };
    let c = exceptions::caught();
    t.check_with(
        "exception-brk",
        c.count == 1 && regs::esr_class(c.esr) == ec::BRK_AARCH64 && regs::esr_iss(c.esr) & 0xffff == 0x42,
        format_args!("count {} esr {:#x}", c.count, c.esr),
    );
    exceptions::expect(ec::SVC_AARCH64);
    // SAFETY: `svc` raises a synchronous exception taken at EL1; the vector
    // table records it and returns to the next instruction.
    unsafe { asm!("svc #0x7") };
    let c = exceptions::caught();
    t.check_with(
        "exception-svc",
        c.count == 2 && regs::esr_class(c.esr) == ec::SVC_AARCH64 && regs::esr_iss(c.esr) & 0xffff == 7,
        format_args!("count {} esr {:#x}", c.count, c.esr),
    );

    let freq = cpu::cntfrq_el0();
    t.check("timer-frequency", freq > 0);
    let start = cpu::counter();
    let mut spins = 0u64;
    while cpu::counter().wrapping_sub(start) < freq / 1000 && spins < 500_000_000 {
        spins += 1;
    }
    let elapsed = cpu::counter().wrapping_sub(start);
    t.check_with("timer-advances", elapsed >= freq / 1000, format_args!("{elapsed} ticks after {spins} spins"));

    // The MMU: identity map of RAM and the devices below it, caches on.
    let (ram_base, ram_size) = ram;
    // SAFETY: called once, on the boot CPU, with the MMU still off; the
    // image, stack, heap (RAM) and the UART (the device gigabyte) are all in
    // the ranges `enable` maps.
    let mapped = unsafe { paging::enable(ram_base, ram_base + ram_size) };
    t.check_with("mmu-enable", mapped.is_ok() && paging::enabled(), format_args!("{mapped:?}"));
    kprint!("# the console works with the MMU on: ");
    kprintln!("{:#x}", uart::base());
    let probe = Box::new(0x1234_5678_9abc_def0u64);
    t.check("mmu-ram-readwrite", *probe == 0x1234_5678_9abc_def0);
    drop(probe);
    match a64_mmu::first_unmapped_address(ram_base + ram_size) {
        Some(hole) => {
            exceptions::expect(ec::DATA_ABORT_SAME);
            // SAFETY: the address is in no mapped gigabyte, so the load
            // faults with a translation fault; the vector table records it
            // and skips this one 4-byte instruction (the result is unused).
            unsafe { asm!("ldr {value}, [{addr}]", addr = in(reg) hole, value = out(reg) _) };
            let c = exceptions::caught();
            let fsc = regs::esr_iss(c.esr);
            t.check_with(
                "mmu-translation-fault",
                c.count == 3 && c.far == hole && regs::is_translation_fault(fsc),
                format_args!("count {} far {:#x} esr {:#x}", c.count, c.far, c.esr),
            );
        }
        None => t.check("mmu-translation-fault", false),
    }

    // Frames and the heap.
    let before = heap::free_frames();
    t.check_with("frames-available", before > 0, format_args!("{before} free"));
    let frame = heap::allocate_frame();
    let frame_ok = frame.is_some_and(|f| {
        f.is_multiple_of(4096) && f >= ram_base && f < ram_base + ram_size && !(image_start..image_end).contains(&f)
    });
    t.check("frames-allocate", frame_ok && heap::free_frames() == before - 1);
    t.check("frames-free", frame.is_some_and(heap::free_frame) && heap::free_frames() == before);

    let in_use = heap::bytes_in_use();
    let mut numbers: Vec<u64> = (0..1000).map(|n| n * n).collect();
    numbers.push(7);
    let text: String = format!("{}-{}", numbers.len(), numbers[999]);
    t.check_with("heap-small", text == "1001-998001" && heap::bytes_in_use() > in_use, format_args!("{text}"));
    drop(numbers);
    drop(text);
    t.check("heap-small-freed", heap::bytes_in_use() == in_use);

    let frames = heap::free_frames();
    // 1 MiB of i % 251: 4177 whole cycles (each 0 + 1 + ... + 250 = 31375)
    // and the first 149 values of another (0 + ... + 148 = 11026).
    const EXPECTED_SUM: u64 = 4177 * 31375 + 11026;
    let big: Vec<u8> = (0..(1usize << 20)).map(|i| (i % 251) as u8).collect();
    let sum: u64 = big.iter().map(|&b| u64::from(b)).sum();
    t.check_with("heap-large", big.len() == 1 << 20 && sum == EXPECTED_SUM, format_args!("sum {sum}"));
    drop(big);
    t.check("heap-large-freed", heap::free_frames() == frames);

    t.tally
}
