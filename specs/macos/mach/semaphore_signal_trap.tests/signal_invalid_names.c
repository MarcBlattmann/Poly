/* semaphore_signal_trap, Mach trap 33 (name): a name that is not in the task is KERN_INVALID_NAME (15): MACH_PORT_NULL, MACH_PORT_DEAD, an unknown name; the call returns at once (it does not wait).
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
    u64 t0 = poly_mach_now();
    check("MACH_PORT_NULL: KERN_INVALID_NAME", (u32)poly_mach6(MACH_semaphore_signal_trap, 0, 0, 0, 0, 0, 0) == KERN_INVALID_NAME, 0);
    check("MACH_PORT_DEAD: KERN_INVALID_NAME", (u32)poly_mach6(MACH_semaphore_signal_trap, MACH_PORT_DEAD, 0, 0, 0, 0, 0) == KERN_INVALID_NAME, 0);
    check("an unknown name: KERN_INVALID_NAME", (u32)poly_mach6(MACH_semaphore_signal_trap, 0x12345, 0, 0, 0, 0, 0) == KERN_INVALID_NAME, 0);
    check("it returned at once (under 50 ms)", poly_mach_now() - t0 < 24000 * 50, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
