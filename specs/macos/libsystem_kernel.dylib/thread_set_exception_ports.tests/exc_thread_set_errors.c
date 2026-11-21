/* poly-mac: libsystem
 * What thread_set_exception_ports refuses: a behavior that is none, a flavor that is none, a mask with bits that are no exception:
 * KERN_INVALID_ARGUMENT (4); the task port instead of a thread port: KERN_INVALID_ARGUMENT; an invalid thread (MACH_PORT_NULL, a name that
 * is nothing): MACH_SEND_INVALID_DEST (0x10000003). A refused call changes nothing. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t th = mach_thread_self();
    mach_port_t p = exc_new_port();
    exception_mask_t m = EXC_MASK_BREAKPOINT;
    check("behavior 99: KERN_INVALID_ARGUMENT", thread_set_exception_ports(th, m, p, 99, EXC_FLAVOR) == KERN_INVALID_ARGUMENT, 0);
    check("flavor 999 with EXCEPTION_STATE: KERN_INVALID_ARGUMENT", thread_set_exception_ports(th, m, p, EXCEPTION_STATE, 999) == KERN_INVALID_ARGUMENT, 0);
    check("mask 0x80000000: KERN_INVALID_ARGUMENT", thread_set_exception_ports(th, 0x80000000, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_INVALID_ARGUMENT, 0);
    check("MACH_PORT_NULL as the thread: MACH_SEND_INVALID_DEST", thread_set_exception_ports(MACH_PORT_NULL, m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == MACH_SEND_INVALID_DEST, 0);
    check("a name that is nothing as the thread: MACH_SEND_INVALID_DEST", thread_set_exception_ports(0x1234, m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == MACH_SEND_INVALID_DEST, 0);
    check("the task port as the thread: KERN_INVALID_ARGUMENT", thread_set_exception_ports(mach_task_self(), m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_INVALID_ARGUMENT, 0);
    exc_entry_t e[14];
    check("nothing was set by the failed calls", exc_get(th, 1, EXC_MASK_ALL, e) == 0, 0);
    check("the same call with good arguments works", thread_set_exception_ports(th, m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_SUCCESS && exc_get(th, 1, m, e) == 1 && e[0].port == p, 0);
    return finish();
}
