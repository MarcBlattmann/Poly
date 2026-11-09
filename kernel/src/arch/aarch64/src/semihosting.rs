//! Ending the run with a status QEMU hands to the host: Arm semihosting
//! `SYS_EXIT` (`-semihosting-config enable=on,target=native`), which the boot
//! tests use because, unlike PSCI `SYSTEM_OFF`, it carries an exit status.
//! Without semihosting (a plain `cargo xtask aarch64 run` still enables it, but
//! another loader may not) the `HLT` raises an exception; the stub then falls
//! back to PSCI `SYSTEM_OFF`, and finally to parking the CPU.

use crate::cpu;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use poly_a64::psci::{self, Conduit};
use poly_a64::semihost;

static EXITING: AtomicBool = AtomicBool::new(false);
static CONDUIT: AtomicU8 = AtomicU8::new(0);

/// Remember how to call PSCI (from the devicetree).
pub fn set_conduit(conduit: Conduit) {
    CONDUIT.store(if conduit == Conduit::Smc { 1 } else { 0 }, Ordering::Relaxed);
}

/// Stop the machine with exit status `code` ([`semihost::status`]).
pub fn exit(code: u32) -> ! {
    // The `HLT` below traps if nothing handles semihosting; the exception
    // handler calls `exit` again, which then skips it.
    if !EXITING.swap(true, Ordering::SeqCst) {
        let block = semihost::exit_block(code);
        // SAFETY: `HLT #0xF000` is the A64 semihosting call: `w0` is the
        // operation, `x1` the address of its parameter block, which lives
        // on this stack frame for the call. The host either ends the
        // machine or, with semihosting off, the instruction is an
        // exception that comes back through the vector table.
        unsafe { asm!("hlt #0xf000", in("x0") semihost::SYS_EXIT, in("x1") block.as_ptr(), options(nostack)) };
    }
    system_off();
    cpu::park()
}

/// PSCI `SYSTEM_OFF` (Arm DEN0022), through the conduit the devicetree names.
fn system_off() {
    // SAFETY: `SYSTEM_OFF` does not return on success; if the conduit is
    // wrong the call is an exception (EL2: an HVC with no handler, EL3: an
    // SMC), which ends in the vector table and then parks. No memory is
    // touched.
    unsafe {
        if CONDUIT.load(Ordering::Relaxed) == 1 {
            asm!("smc #0", in("x0") psci::SYSTEM_OFF, options(nostack));
        } else {
            asm!("hvc #0", in("x0") psci::SYSTEM_OFF, options(nostack));
        }
    }
}
