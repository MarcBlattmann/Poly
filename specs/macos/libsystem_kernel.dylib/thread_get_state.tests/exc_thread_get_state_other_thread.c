/* poly-mac: libsystem
 * The state of another thread of the task: while it is suspended (thread_suspend) two reads give the same registers and a pc that is not 0; the
 * state is also given by the exception handler's read of the faulting thread: a handler that reads the state of the thread that raised a breakpoint
 * sees the register the thread set just before it (0x42) and a pc inside the program. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t other;
    pthread_t w = exc_start_waiter(&other);
    check("thread_suspend succeeds", thread_suspend(other) == KERN_SUCCESS, 0);
    exc_ts_t a, b;
    mach_msg_type_number_t n = EXC_STATE_COUNT, m = EXC_STATE_COUNT;
    kern_return_t k1 = thread_get_state(other, EXC_FLAVOR, (thread_state_t)&a, &n);
    kern_return_t k2 = thread_get_state(other, EXC_FLAVOR, (thread_state_t)&b, &m);
    check("two reads succeed", k1 == KERN_SUCCESS && k2 == KERN_SUCCESS, k1 ? k1 : k2);
    check("the pc is not 0", exc_get_pc(&a) != 0, 0);
    check("a suspended thread gives the same state twice", memcmp(&a, &b, sizeof a) == 0, 0);
    check("thread_resume succeeds", thread_resume(other) == KERN_SUCCESS, 0);
    exc_stop_waiter(w);
    exc_handler_t h;
    exc_handler_start(&h);
    task_set_exception_ports(mach_task_self(), EXC_MASK_BREAKPOINT, h.port, EXCEPTION_DEFAULT, EXC_FLAVOR);
    exc_run(&h, exc_fault_break);
    check("the handler saw the register the thread had set before the breakpoint", h.seen.reg == 0x42, (long long)h.seen.reg);
    check("and a pc that is not 0", h.seen.pc != 0, 0);
    return finish();
}
