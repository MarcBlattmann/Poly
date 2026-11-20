/* poly-mac: libsystem
 * What thread_set_state refuses: a count smaller than the flavor's and a flavor that is none (999; not asked on x86_64, where Rosetta 2 ends the program): KERN_INVALID_ARGUMENT (4); an invalid thread
 * (MACH_PORT_NULL): MACH_SEND_INVALID_DEST (0x10000003); a task port instead of a thread: KERN_INVALID_ARGUMENT. Nothing is changed by a refused call. */
#include "poly_mexc.h"
int test_main(void) {
    mach_port_t other;
    pthread_t w = exc_start_waiter(&other);
    thread_suspend(other);
    exc_ts_t s;
    mach_msg_type_number_t n = EXC_STATE_COUNT;
    thread_get_state(other, EXC_FLAVOR, (thread_state_t)&s, &n);
    check("a count one too small: KERN_INVALID_ARGUMENT", thread_set_state(other, EXC_FLAVOR, (thread_state_t)&s, EXC_STATE_COUNT - 1) == KERN_INVALID_ARGUMENT, 0);
#if defined(__arm64__)
    /* under Rosetta 2 a flavor that is none ends the program with an error message of the translator, so it is not asked there */
    check("flavor 999: KERN_INVALID_ARGUMENT", thread_set_state(other, 999, (thread_state_t)&s, EXC_STATE_COUNT) == KERN_INVALID_ARGUMENT, 0);
#endif
    check("MACH_PORT_NULL as the thread: MACH_SEND_INVALID_DEST", thread_set_state(MACH_PORT_NULL, EXC_FLAVOR, (thread_state_t)&s, EXC_STATE_COUNT) == MACH_SEND_INVALID_DEST, 0);
    check("the task port as the thread: KERN_INVALID_ARGUMENT", thread_set_state(mach_task_self(), EXC_FLAVOR, (thread_state_t)&s, EXC_STATE_COUNT) == KERN_INVALID_ARGUMENT, 0);
    thread_resume(other);
    exc_stop_waiter(w);
    return finish();
}
