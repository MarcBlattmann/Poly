/* poly-mac: libsystem
 * task_swap_exception_ports(task, mask, port, behavior, flavor, old...) does what task_set_exception_ports does and returns the entries it
 * replaced, as task_get_exception_ports would have given them just before: for exceptions that had a port the port, behavior and flavor; for
 * those that had none an entry with a null port; the old entries cover only the mask given. Swapping in MACH_PORT_NULL removes the port. */
#include "poly_mexc.h"
static int swap(mach_port_t t, exception_mask_t mask, mach_port_t port, exception_behavior_t b, thread_state_flavor_t f, exc_entry_t out[14]) {
    exception_mask_t m[EXC_TYPES_COUNT];
    mach_port_t p[EXC_TYPES_COUNT];
    exception_behavior_t bb[EXC_TYPES_COUNT];
    thread_state_flavor_t ff[EXC_TYPES_COUNT];
    mach_msg_type_number_t n = EXC_TYPES_COUNT;
    kern_return_t kr = task_swap_exception_ports(t, mask, port, b, f, m, &n, p, bb, ff);
    if (kr) return -1 - kr;
    for (unsigned i = 0; i < n; i++) {
        out[i].mask = m[i];
        out[i].port = p[i];
        out[i].behavior = bb[i];
        out[i].flavor = ff[i];
    }
    return (int)n;
}
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t a = exc_new_port(), b = exc_new_port();
    exc_entry_t old[14], e[14];
    int n = swap(t, EXC_MASK_BAD_ACCESS | EXC_MASK_ARITHMETIC, a, EXCEPTION_STATE_IDENTITY, EXC_FLAVOR, old);
    check("swap on exceptions that had no port: one old entry covering the mask, with a null port", n == 1 && old[0].mask == (EXC_MASK_BAD_ACCESS | EXC_MASK_ARITHMETIC) && old[0].port == MACH_PORT_NULL, n);
    n = swap(t, EXC_MASK_BAD_ACCESS, b, EXCEPTION_DEFAULT, THREAD_STATE_NONE, old);
    check("swap on one exception that had port a: the old entry is a's, for that exception only", n == 1 && old[0].mask == EXC_MASK_BAD_ACCESS && old[0].port == a && old[0].behavior == EXCEPTION_STATE_IDENTITY && old[0].flavor == EXC_FLAVOR, n);
    n = exc_get(t, 0, EXC_MASK_BAD_ACCESS | EXC_MASK_ARITHMETIC, e);
    exc_entry_t *x = exc_entry_for(e, n, EXC_MASK_BAD_ACCESS), *y = exc_entry_for(e, n, EXC_MASK_ARITHMETIC);
    check("afterwards access has port b and arithmetic still port a", n == 2 && x && y && x->port == b && y->port == a && x->behavior == EXCEPTION_DEFAULT, n);
    n = swap(t, EXC_MASK_BAD_ACCESS | EXC_MASK_ARITHMETIC, a, EXCEPTION_DEFAULT, THREAD_STATE_NONE, old);
    check("a swap across two ports: two old entries, one each", n == 2, n);
    unsigned u0 = exc_send_urefs(b);
    swap(t, EXC_MASK_BREAKPOINT, MACH_PORT_NULL, EXCEPTION_DEFAULT, THREAD_STATE_NONE, old);
    n = swap(t, EXC_MASK_BAD_ACCESS, MACH_PORT_NULL, EXCEPTION_DEFAULT, THREAD_STATE_NONE, old);
    check("swapping in MACH_PORT_NULL returns the old port and leaves a null one", n == 1 && old[0].port == a && exc_get(t, 0, EXC_MASK_BAD_ACCESS, e) == 1 && e[0].port == MACH_PORT_NULL, n);
    check("a swap hands out user references to the old ports", exc_send_urefs(a) > 1 && exc_send_urefs(b) >= u0, exc_send_urefs(a));
    return finish();
}
