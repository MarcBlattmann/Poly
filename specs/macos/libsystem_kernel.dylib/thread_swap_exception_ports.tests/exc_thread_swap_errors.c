/* poly-mac: libsystem
 * thread_swap_exception_ports refuses what thread_set_exception_ports refuses (a behavior or flavor that is none, a mask with bits that are no exception:
 * KERN_INVALID_ARGUMENT (4); the task port instead of a thread port: KERN_INVALID_ARGUMENT; an invalid thread: MACH_SEND_INVALID_DEST), and a refused
 * call changes nothing. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t th = mach_thread_self();
    mach_port_t a = exc_new_port(), b = exc_new_port();
    thread_set_exception_ports(th, EXC_MASK_BREAKPOINT, a, EXCEPTION_DEFAULT, EXC_FLAVOR);
    exception_mask_t m[EXC_TYPES_COUNT];
    mach_port_t p[EXC_TYPES_COUNT];
    exception_behavior_t bb[EXC_TYPES_COUNT];
    thread_state_flavor_t ff[EXC_TYPES_COUNT];
    mach_msg_type_number_t n = EXC_TYPES_COUNT;
    check("behavior 99: KERN_INVALID_ARGUMENT", thread_swap_exception_ports(th, EXC_MASK_BREAKPOINT, b, 99, EXC_FLAVOR, m, &n, p, bb, ff) == KERN_INVALID_ARGUMENT, 0);
    n = EXC_TYPES_COUNT;
    check("flavor 999: KERN_INVALID_ARGUMENT", thread_swap_exception_ports(th, EXC_MASK_BREAKPOINT, b, EXCEPTION_STATE, 999, m, &n, p, bb, ff) == KERN_INVALID_ARGUMENT, 0);
    n = EXC_TYPES_COUNT;
    check("mask 0x80000000: KERN_INVALID_ARGUMENT", thread_swap_exception_ports(th, 0x80000000, b, EXCEPTION_DEFAULT, EXC_FLAVOR, m, &n, p, bb, ff) == KERN_INVALID_ARGUMENT, 0);
    n = EXC_TYPES_COUNT;
    check("the task port as the thread: KERN_INVALID_ARGUMENT", thread_swap_exception_ports(mach_task_self(), EXC_MASK_BREAKPOINT, b, EXCEPTION_DEFAULT, EXC_FLAVOR, m, &n, p, bb, ff) == KERN_INVALID_ARGUMENT, 0);
    n = EXC_TYPES_COUNT;
    check("MACH_PORT_NULL as the thread: MACH_SEND_INVALID_DEST", thread_swap_exception_ports(MACH_PORT_NULL, EXC_MASK_BREAKPOINT, b, EXCEPTION_DEFAULT, EXC_FLAVOR, m, &n, p, bb, ff) == MACH_SEND_INVALID_DEST, 0);
    exc_entry_t e[14];
    int cnt = exc_get(th, 1, EXC_MASK_BREAKPOINT, e);
    check("the earlier setting is unchanged", cnt == 1 && e[0].port == a, cnt);
    return finish();
}
