/* poly-mac: libsystem
 * task_set_exception_ports(task, mask, port, behavior, flavor) puts the port in for each exception of the mask; task_get_exception_ports
 * returns it as it was set: the behavior (EXCEPTION_DEFAULT 1, STATE 2, STATE_IDENTITY 3, with MACH_EXCEPTION_CODES 0x80000000 kept) and the
 * flavor (for DEFAULT the flavor given too, THREAD_STATE_NONE or the architecture's thread state); exceptions set with the same
 * port, behavior and flavor are reported in one entry; each call adds a user reference to the port's send right; MACH_PORT_NULL takes
 * the port away again (the entry stays, with a null port); a mask of 0 changes nothing. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = exc_new_port();
    unsigned u0 = exc_send_urefs(p);
    exception_mask_t mask = EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT;
    exc_entry_t e[14];
    int behaviors[] = {EXCEPTION_DEFAULT, EXCEPTION_STATE, EXCEPTION_STATE_IDENTITY, EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES, EXCEPTION_STATE | MACH_EXCEPTION_CODES, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES};
    for (unsigned i = 0; i < 6; i++) {
        kern_return_t kr = task_set_exception_ports(t, mask, p, behaviors[i], EXC_FLAVOR);
        int n = exc_get(t, 0, mask, e);
        char name[64];
        snprintf(name, sizeof name, "behavior %#x: set succeeds and reads back in one entry", (unsigned)behaviors[i]);
        check(name, kr == KERN_SUCCESS && n == 1 && e[0].mask == mask && e[0].port == p && (unsigned)e[0].behavior == (unsigned)behaviors[i] && e[0].flavor == EXC_FLAVOR, n);
    }
    check("each set added a user reference to the send right", exc_send_urefs(p) >= u0 + 6, exc_send_urefs(p));
    task_set_exception_ports(t, mask, p, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
    int n = exc_get(t, 0, mask, e);
    check("flavor THREAD_STATE_NONE reads back as it was given", n == 1 && e[0].flavor == EXC_NONE_FLAVOR, e[0].flavor);
    value("the flavor value of THREAD_STATE_NONE here", EXC_NONE_FLAVOR);
    mach_port_t q = exc_new_port();
    task_set_exception_ports(t, EXC_MASK_BREAKPOINT, q, EXCEPTION_STATE, EXC_FLAVOR);
    n = exc_get(t, 0, mask, e);
    check("two ports for the two exceptions: two entries", n == 2, n);
    exc_entry_t *a = exc_entry_for(e, n, EXC_MASK_BAD_ACCESS), *b = exc_entry_for(e, n, EXC_MASK_BREAKPOINT);
    check("each with its own port and behavior", a && b && a->port == p && b->port == q && a->behavior == EXCEPTION_DEFAULT && b->behavior == EXCEPTION_STATE, 0);
    check("a mask of 0 changes nothing", task_set_exception_ports(t, 0, q, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_SUCCESS && exc_get(t, 0, mask, e) == 2, 0);
    kern_return_t kr = task_set_exception_ports(t, mask, MACH_PORT_NULL, EXCEPTION_DEFAULT, EXC_FLAVOR);
    n = exc_get(t, 0, mask, e);
    check("MACH_PORT_NULL takes the ports away: one entry with a null port", kr == KERN_SUCCESS && n == 1 && e[0].port == MACH_PORT_NULL && e[0].mask == mask, n);
    return finish();
}
