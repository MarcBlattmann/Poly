/* semaphore_timedwait_trap, Mach trap 38: the time is checked before the name: nanoseconds of 1000000000 or more (2e9) or negative are KERN_INVALID_VALUE (18) even for a name that does not exist; a negative number of seconds with 0 nanoseconds is not refused (the name error follows: KERN_INVALID_NAME, or KERN_INVALID_RIGHT for a receive right).
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
    check("nanoseconds 2e9, unknown name: KERN_INVALID_VALUE", (u32)poly_mach6(MACH_semaphore_timedwait_trap, 0x12345, 0, 2000000000, 0, 0, 0) == KERN_INVALID_VALUE, 0);
    check("nanoseconds -1, unknown name: KERN_INVALID_VALUE", (u32)poly_mach6(MACH_semaphore_timedwait_trap, 0x12345, 0, -1, 0, 0, 0) == KERN_INVALID_VALUE, 0);
    check("nanoseconds 2e9, a receive right: KERN_INVALID_VALUE", (u32)poly_mach6(MACH_semaphore_timedwait_trap, rp, 0, 2000000000, 0, 0, 0) == KERN_INVALID_VALUE, 0);
    check("seconds -1, nanoseconds 0, unknown name: KERN_INVALID_NAME", (u32)poly_mach6(MACH_semaphore_timedwait_trap, 0x12345, -1, 0, 0, 0, 0) == KERN_INVALID_NAME, 0);
    check("seconds -1, nanoseconds 0, a receive right: KERN_INVALID_RIGHT", (u32)poly_mach6(MACH_semaphore_timedwait_trap, rp, -1, 0, 0, 0, 0) == KERN_INVALID_RIGHT, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
