/* mach_msg_trap, Mach trap 31 (message, option, send size, receive size, receive port, timeout in ms, 0): a receive with MACH_RCV_TIMEOUT (0x100) on a port with no message fails with MACH_RCV_TIMED_OUT (0x10004003) after the timeout (at least 15 ms went by for 20 ms; 24 MHz ticks), at once for a timeout of 0; the same for a port set, with MACH_RCV_LARGE and with a trailer option; the receive size and the buffer do not matter while nothing is queued.
 * mach_msg_trap has seven arguments, which the test header passes on arm64
 * only: on x86_64 this test checks nothing yet. */
#include "poly_mtest.h"

#if defined(POLY_ARM64)
static __attribute__((unused)) i64 self(void) { return mach0(MACH_task_self_trap); }

static u32 m[64];

int test_main(void) {
    i64 task = self();
    u32 p = 0;
    poly_mach6(MACH__kernelrpc_mach_port_allocate_trap, task, MACH_PORT_RIGHT_RECEIVE, (i64)&p, 0, 0, 0);
    u64 t0 = poly_mach_now();
    check("20 ms on an empty port: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof m, p, 20) == MACH_RCV_TIMED_OUT, 0);
    u64 t1 = poly_mach_now();
    check("it took at least 15 ms and less than 500 ms", t1 - t0 >= 24000 * 15 && t1 - t0 < 24000 * 500, (i64)(t1 - t0));
    u64 t2 = poly_mach_now();
    check("a timeout of 0: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof m, p, 0) == MACH_RCV_TIMED_OUT, 0);
    check("and at once (under 5 ms)", poly_mach_now() - t2 < 24000 * 5, 0);
    u32 set = 0;
    poly_mach6(MACH__kernelrpc_mach_port_allocate_trap, task, MACH_PORT_RIGHT_PORT_SET, (i64)&set, 0, 0, 0);
    check("an empty port set: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof m, set, 5) == MACH_RCV_TIMED_OUT, 0);
    check("with MACH_RCV_LARGE: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT | MACH_RCV_LARGE, 0, sizeof m, p, 5) == MACH_RCV_TIMED_OUT, 0);
    u32 trailer = MACH_RCV_TRAILER_TYPE(0) | MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT);
    check("with an audit trailer asked for: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT | trailer, 0, sizeof m, p, 5) == MACH_RCV_TIMED_OUT, 0);
    check("a receive size of 0: MACH_RCV_TIMED_OUT", poly_mach_msg(m, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, 0, p, 5) == MACH_RCV_TIMED_OUT, 0);
    check("a buffer at address 8: MACH_RCV_TIMED_OUT", poly_mach_msg((void *)8, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, 128, p, 5) == MACH_RCV_TIMED_OUT, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
