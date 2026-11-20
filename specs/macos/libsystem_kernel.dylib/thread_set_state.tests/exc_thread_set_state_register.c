/* poly-mac: libsystem
 * thread_set_state on a thread stopped by an exception (the handler's) with a register changed: the thread continues with that register once the
 * handler replies KERN_SUCCESS (breakpoint: the thread had set 0x42 and then stores the register: it stores what the handler wrote). The
 * register is x9 on arm64 and rax on x86_64. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h;
    exc_handler_start(&h);
    task_set_exception_ports(mach_task_self(), EXC_MASK_BREAKPOINT, h.port, EXCEPTION_DEFAULT, EXC_FLAVOR);
    h.set_reg = 1;
    h.reg_val = 0xfedcba9876543210UL;
    exc_reg_out = 0;
    exc_run(&h, exc_fault_break);
    check("one exception handled", h.count == 1, h.count);
    check("the thread stored the full 64-bit register the handler wrote", exc_reg_out == 0xfedcba9876543210UL, (long long)exc_reg_out);
    h.set_reg = 0;
    exc_reg_out = 0;
    exc_run(&h, exc_fault_break);
    check("without a change the thread keeps its own value, 0x42", exc_reg_out == 0x42, (long long)exc_reg_out);
    h.set_reg = 1;
    h.reg_val = 0;
    exc_run(&h, exc_fault_break);
    check("a register can be set to 0", exc_reg_out == 0, (long long)exc_reg_out);
    return finish();
}
