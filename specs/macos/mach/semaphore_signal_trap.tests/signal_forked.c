/* semaphore_signal_trap, Mach trap 33: also in a forked child (which has no semaphores of the parent): the same errors, the same results.
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
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        i64 t = self();
        u32 r = (u32)mach0(MACH_mach_reply_port);
        int ok = (u32)poly_mach6(MACH_semaphore_signal_trap, 0x12345, 0, 0, 0, 0, 0) == KERN_INVALID_NAME
              && (u32)poly_mach6(MACH_semaphore_signal_trap, t, 0, 0, 0, 0, 0) == KERN_INVALID_CAPABILITY
              && (u32)poly_mach6(MACH_semaphore_signal_trap, r, 0, 0, 0, 0, 0) == KERN_INVALID_RIGHT;
        poly_exit(ok ? 0 : 1);
    }
    int st = poly_wait(f.ret);
    check("the child got the same three errors", W_EXITED(st) && W_EXITSTATUS(st) == 0, st);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
