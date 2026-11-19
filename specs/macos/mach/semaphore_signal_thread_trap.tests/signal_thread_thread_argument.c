/* semaphore_signal_thread_trap, Mach trap 35 (semaphore name, thread name): a thread name that is not 0 and not a thread port (an unknown name, the task's port, a receive right) is KERN_INVALID_ARGUMENT (4), whatever the semaphore name is; a thread port (or 0) leaves the semaphore name's error: KERN_INVALID_NAME, KERN_INVALID_CAPABILITY or KERN_INVALID_RIGHT as for the other traps.
 * A semaphore port can only be made by sending a message to the task port,
 * and mach_msg_trap cannot send on this platform, so what is specified here
 * is what the trap refuses. */
#include "poly_mtest.h"

#if defined(POLY_ARM64)
static __attribute__((unused)) i64 self(void) { return mach0(MACH_task_self_trap); }

int test_main(void) {
    i64 task = self();
    u32 rp = (u32)mach0(MACH_mach_reply_port), th = (u32)mach0(MACH_thread_self_trap);
    (void)task; (void)rp; (void)th;
    u32 sems[4] = {0, 0x12345, (u32)task, rp};
    u32 sem_err[4] = {KERN_INVALID_NAME, KERN_INVALID_NAME, KERN_INVALID_CAPABILITY, KERN_INVALID_RIGHT};
    u32 bad_threads[3] = {0x12345, (u32)task, rp};
    int bad_ok = 1, good_ok = 1;
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j < 3; j++) bad_ok &= (u32)poly_mach6(MACH_semaphore_signal_thread_trap, sems[i], bad_threads[j], 0, 0, 0, 0) == KERN_INVALID_ARGUMENT;
        good_ok &= (u32)poly_mach6(MACH_semaphore_signal_thread_trap, sems[i], th, 0, 0, 0, 0) == sem_err[i];
        good_ok &= (u32)poly_mach6(MACH_semaphore_signal_thread_trap, sems[i], 0, 0, 0, 0, 0) == sem_err[i];
    }
    check("a thread name that is not a thread: KERN_INVALID_ARGUMENT for every semaphore name", bad_ok, 0);
    check("a thread port or 0: the semaphore name's own error", good_ok, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
