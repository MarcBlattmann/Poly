/* poly-mac: libsystem
 * Thread ports are per thread: what is set on this thread is not there on another thread of the same task, and what is set on the task is
 * not in a thread's entries (and the other way round); a thread made after a task port was set does not copy it into its own entries. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t me = mach_thread_self(), other;
    mach_port_t tk = exc_new_port(), thp = exc_new_port();
    task_set_exception_ports(mach_task_self(), EXC_MASK_BREAKPOINT, tk, EXCEPTION_DEFAULT, EXC_FLAVOR);
    pthread_t w = exc_start_waiter(&other);
    exc_entry_t e[14];
    check("a thread made after the task's port was set has no entry of its own", exc_get(other, 1, EXC_MASK_ALL, e) == 0, 0);
    thread_set_exception_ports(me, EXC_MASK_BREAKPOINT, thp, EXCEPTION_DEFAULT, EXC_FLAVOR);
    check("this thread's entry is its own port", exc_get(me, 1, EXC_MASK_BREAKPOINT, e) == 1 && e[0].port == thp, 0);
    check("the other thread still has none", exc_get(other, 1, EXC_MASK_ALL, e) == 0, 0);
    check("the task's entry is still its own port", exc_get(mach_task_self(), 0, EXC_MASK_BREAKPOINT, e) == 1 && e[0].port == tk, 0);
    exc_stop_waiter(w);
    return finish();
}
