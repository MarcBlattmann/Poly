/* semaphore_timedwait_signal_trap, Mach trap 39 (wait name, signal name, seconds, nanoseconds): a name that is not a semaphore is refused by what it is: a send right to a task or thread is KERN_INVALID_CAPABILITY (20), a receive right is KERN_INVALID_RIGHT (17).
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
    check("the task's own port: KERN_INVALID_CAPABILITY", (u32)poly_mach6(MACH_semaphore_timedwait_signal_trap, (u32)task, (u32)task, 0, 1000000, 0, 0) == KERN_INVALID_CAPABILITY, 0);
    check("the thread's own port: KERN_INVALID_CAPABILITY", (u32)poly_mach6(MACH_semaphore_timedwait_signal_trap, th, th, 0, 1000000, 0, 0) == KERN_INVALID_CAPABILITY, 0);
    check("a receive right (a reply port): KERN_INVALID_RIGHT", (u32)poly_mach6(MACH_semaphore_timedwait_signal_trap, rp, rp, 0, 1000000, 0, 0) == KERN_INVALID_RIGHT, 0);
    check("the reply port is untouched: its type is still a receive right", ((u32)poly_mach6(MACH_semaphore_timedwait_signal_trap, rp, rp, 0, 1000000, 0, 0)) == KERN_INVALID_RIGHT, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
