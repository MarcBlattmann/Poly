/* poly-mac: libsystem
 * After thread_swap_exception_ports a fault of that thread goes to the new port's handler, not the old one's, and the thread continues when the new
 * handler resumes it. */
#include "poly_mexc.h"
static volatile int go;
static mach_port_t thread_port;
static void *waiter_then_break(void *a) {
    (void)a;
    thread_port = mach_thread_self();
    while (!go) usleep(1000);
    return exc_fault_break(NULL);
}
int test_main(void) {
    exc_handler_t h1, h2;
    exc_handler_start(&h1);
    exc_handler_start(&h2);
    pthread_t t;
    pthread_create(&t, NULL, waiter_then_break, NULL);
    while (!thread_port) usleep(1000);
    exc_current = &h2;
    thread_set_exception_ports(thread_port, EXC_MASK_BREAKPOINT, h1.port, EXCEPTION_DEFAULT, EXC_FLAVOR);
    exception_mask_t m[EXC_TYPES_COUNT];
    mach_port_t p[EXC_TYPES_COUNT];
    exception_behavior_t bb[EXC_TYPES_COUNT];
    thread_state_flavor_t ff[EXC_TYPES_COUNT];
    mach_msg_type_number_t n = EXC_TYPES_COUNT;
    kern_return_t kr = thread_swap_exception_ports(thread_port, EXC_MASK_BREAKPOINT, h2.port, EXCEPTION_DEFAULT, EXC_FLAVOR, m, &n, p, bb, ff);
    check("the swap succeeds and returns the old port", kr == KERN_SUCCESS && n == 1 && p[0] == h1.port, kr);
    h2.set_reg = 1;
    h2.reg_val = 0x77;
    go = 1;
    pthread_join(t, NULL);
    check("the breakpoint went to the new handler", h2.count == 1 && h1.count == 0, h2.count * 10 + h1.count);
    check("and the thread continued with the register that handler set", exc_reg_out == 0x77, (long long)exc_reg_out);
    return finish();
}
