/* poly-mac: libsystem
 * After task_swap_exception_ports the new port gets the exceptions: a fault is delivered to the new port's handler and not to the old one's; the
 * handler of the old port, which is the one the swap returned, still works when asked again with a set. */
#include "poly_mexc.h"
int test_main(void) {
    exc_handler_t h1, h2;
    exc_handler_start(&h1);
    exc_handler_start(&h2);
    mach_port_t t = mach_task_self();
    task_set_exception_ports(t, EXC_MASK_BREAKPOINT, h1.port, EXCEPTION_DEFAULT, EXC_FLAVOR);
    exception_mask_t m[EXC_TYPES_COUNT];
    mach_port_t p[EXC_TYPES_COUNT];
    exception_behavior_t bb[EXC_TYPES_COUNT];
    thread_state_flavor_t ff[EXC_TYPES_COUNT];
    mach_msg_type_number_t n = EXC_TYPES_COUNT;
    kern_return_t kr = task_swap_exception_ports(t, EXC_MASK_BREAKPOINT, h2.port, EXCEPTION_DEFAULT, EXC_FLAVOR, m, &n, p, bb, ff);
    check("the swap succeeds and returns the old port", kr == KERN_SUCCESS && n == 1 && p[0] == h1.port, kr);
    exc_run(&h2, exc_fault_break);
    check("the breakpoint is handled by the new port's handler", h2.count == 1 && h1.count == 0, h2.count);
    check("and the thread continued", exc_reg_out == 0x42, (long long)exc_reg_out);
    task_set_exception_ports(t, EXC_MASK_BREAKPOINT, h1.port, EXCEPTION_DEFAULT, EXC_FLAVOR);
    exc_run(&h1, exc_fault_break);
    check("set back to the old port: its handler gets the next one", h1.count == 1 && h2.count == 1, h1.count);
    return finish();
}
