/* poly-mac: libsystem
 * What task_get_exception_ports reports after sets: one entry per distinct (port, behavior, flavor) among the exceptions of the mask, plus one for
 * those with no port; an entry's mask holds only exceptions that were asked for; the entries come in the order of the lowest exception bit
 * of each; each call returns send rights to the ports (a user reference more each time). */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t a = exc_new_port(), b = exc_new_port();
    task_set_exception_ports(t, EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT, a, EXCEPTION_DEFAULT, THREAD_STATE_NONE);
    task_set_exception_ports(t, EXC_MASK_ARITHMETIC | EXC_MASK_SOFTWARE, b, EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES, EXC_FLAVOR);
    exception_mask_t all = EXC_MASK_CLASSIC;
    exc_entry_t e[14];
    int n = exc_get(t, 0, all, e);
    check("three entries: a's, b's and the exceptions with no port", n == 3, n);
    exc_entry_t *ea = exc_entry_for(e, n, EXC_MASK_BAD_ACCESS), *eb = exc_entry_for(e, n, EXC_MASK_ARITHMETIC);
    check("the access and breakpoint exceptions share an entry with port a", ea && ea == exc_entry_for(e, n, EXC_MASK_BREAKPOINT) && ea->port == a && ea->mask == (EXC_MASK_BAD_ACCESS | EXC_MASK_BREAKPOINT), 0);
    check("the arithmetic and software exceptions share one with port b", eb && eb == exc_entry_for(e, n, EXC_MASK_SOFTWARE) && eb->port == b && eb->mask == (EXC_MASK_ARITHMETIC | EXC_MASK_SOFTWARE), 0);
    check("their behaviors as set", ea->behavior == EXCEPTION_DEFAULT && (unsigned)eb->behavior == (EXCEPTION_STATE_IDENTITY | MACH_EXCEPTION_CODES), ea->behavior);
    exc_entry_t *none = exc_entry_for(e, n, EXC_MASK_BAD_INSTRUCTION);
    check("the others have a null port", none && none->port == MACH_PORT_NULL, 0);
    int ordered = 1;
    unsigned prev = 0;
    for (int i = 0; i < n; i++) {
        unsigned low = (unsigned)e[i].mask & (unsigned)-(int)e[i].mask;
        ordered &= low > prev;
        prev = low;
    }
    check("the entries are in the order of their lowest exception bit", ordered, 0);
    int one = exc_get(t, 0, EXC_MASK_BAD_ACCESS, e);
    check("asking for access only: one entry whose mask is just that", one == 1 && e[0].mask == EXC_MASK_BAD_ACCESS && e[0].port == a, one);
    unsigned u0 = exc_send_urefs(a);
    exc_get(t, 0, EXC_MASK_BAD_ACCESS, e);
    exc_get(t, 0, EXC_MASK_BAD_ACCESS, e);
    check("each get adds a user reference to the port's send right", exc_send_urefs(a) == u0 + 2, exc_send_urefs(a));
    return finish();
}
