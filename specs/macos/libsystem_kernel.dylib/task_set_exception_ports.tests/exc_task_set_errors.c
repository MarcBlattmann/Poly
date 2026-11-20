/* poly-mac: libsystem
 * What task_set_exception_ports refuses: a behavior that is none (99) or a state behavior with a flavor that is none (999), a flavor that is none
 * with any behavior, a mask with bits that are no exception (0x80000000): KERN_INVALID_ARGUMENT (4); an invalid task (MACH_PORT_NULL, a
 * name that is nothing): MACH_SEND_INVALID_DEST (0x10000003); the thread version given the task port: KERN_INVALID_ARGUMENT. A state behavior
 * with THREAD_STATE_NONE as the flavor is accepted. What the port argument may be differs between the architectures observed (see the values). */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = exc_new_port();
    exception_mask_t m = EXC_MASK_BAD_ACCESS;
    check("behavior 99: KERN_INVALID_ARGUMENT", task_set_exception_ports(t, m, p, 99, EXC_FLAVOR) == KERN_INVALID_ARGUMENT, 0);
    check("flavor 999 with EXCEPTION_STATE: KERN_INVALID_ARGUMENT", task_set_exception_ports(t, m, p, EXCEPTION_STATE, 999) == KERN_INVALID_ARGUMENT, 0);
    check("flavor 999 with EXCEPTION_DEFAULT: KERN_INVALID_ARGUMENT", task_set_exception_ports(t, m, p, EXCEPTION_DEFAULT, 999) == KERN_INVALID_ARGUMENT, 0);
    check("EXCEPTION_STATE with THREAD_STATE_NONE is accepted", task_set_exception_ports(t, m, p, EXCEPTION_STATE, THREAD_STATE_NONE) == KERN_SUCCESS, 0);
    check("mask 0x80000000: KERN_INVALID_ARGUMENT", task_set_exception_ports(t, 0x80000000, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_INVALID_ARGUMENT, 0);
    check("MACH_PORT_NULL as the task: MACH_SEND_INVALID_DEST", task_set_exception_ports(MACH_PORT_NULL, m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == MACH_SEND_INVALID_DEST, 0);
    check("a name that is nothing as the task: MACH_SEND_INVALID_DEST", task_set_exception_ports(0x1234, m, p, EXCEPTION_DEFAULT, EXC_FLAVOR) == MACH_SEND_INVALID_DEST, 0);
    check("thread_set_exception_ports given the task port: KERN_INVALID_ARGUMENT", thread_set_exception_ports(t, m, p, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == KERN_INVALID_ARGUMENT, 0);
    check("MACH_PORT_DEAD as the port is accepted", task_set_exception_ports(t, m, MACH_PORT_DEAD, EXCEPTION_DEFAULT, EXC_FLAVOR) == KERN_SUCCESS, 0);
    mach_port_t recv_only = MACH_PORT_NULL;
    mach_port_allocate(t, MACH_PORT_RIGHT_RECEIVE, &recv_only);
    value("a port that is a receive right only: kern_return", task_set_exception_ports(t, m, recv_only, EXCEPTION_DEFAULT, EXC_FLAVOR));
    value("a port name that does not exist: kern_return", task_set_exception_ports(t, m, 0x4321, EXCEPTION_DEFAULT, EXC_FLAVOR));
    value("behavior with the bit 0x40000000: kern_return", task_set_exception_ports(t, m, p, EXCEPTION_DEFAULT | 0x40000000, EXC_FLAVOR));
    value("behavior with MACH_EXCEPTION_BACKTRACE_PREFERRED: kern_return", task_set_exception_ports(t, m, p, EXCEPTION_DEFAULT | MACH_EXCEPTION_BACKTRACE_PREFERRED, EXC_FLAVOR));
    return finish();
}
