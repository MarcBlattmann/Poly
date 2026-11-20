/* poly-mac: libsystem
 * thread_set_exception_ports(thread, mask, port, behavior, flavor) sets the ports of one thread (this thread's port or another thread's);
 * thread_get_exception_ports reads them back as set (behavior, flavor, merged masks); the entries are only for exceptions with a port (a
 * fresh thread has none at all, but once it has set a port it reports the other exceptions of the mask with a null port, as the task call
 * does); it does not touch the task's ports or another thread's; MACH_PORT_NULL takes a port away. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t p = exc_new_port();
    mach_port_t other;
    pthread_t w = exc_start_waiter(&other);
    exc_entry_t e[14];
    check("another thread, fresh: no entries", exc_get(other, 1, EXC_MASK_ALL, e) == 0, 0);
    kern_return_t kr = thread_set_exception_ports(other, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, p, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES, EXC_FLAVOR);
    check("set on another thread succeeds", kr == KERN_SUCCESS, kr);
    int n = exc_get(other, 1, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, e);
    check("read back: one entry for the two exceptions, as set", n == 1 && e[0].mask == (EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT) && e[0].port == p && (unsigned)e[0].behavior == (EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES) && e[0].flavor == EXC_FLAVOR, n);
    check("this thread has none", exc_get(mach_thread_self(), 1, EXC_MASK_ALL, e) == 0, 0);
    exc_entry_t t[14];
    int tn = exc_get(mach_task_self(), 0, EXC_MASK_BAD_ACCESS, t);
    check("and the task's port for access is untouched (null)", tn == 1 && t[0].port == MACH_PORT_NULL, tn);
    mach_port_t q = exc_new_port();
    thread_set_exception_ports(other, EXC_MASK_BREAKPOINT, q, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
    n = exc_get(other, 1, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, e);
    check("a second port for one exception: two entries", n == 2, n);
    thread_set_exception_ports(other, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, MACH_PORT_NULL, EXCEPTION_DEFAULT, EXC_FLAVOR);
    n = exc_get(other, 1, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, e);
    check("MACH_PORT_NULL takes the ports away: one entry with a null port", n == 1 && e[0].port == MACH_PORT_NULL && e[0].mask == (EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT), n);
    exc_stop_waiter(w);
    return finish();
}
