/* mach_msg_trap, Mach trap 31, receive errors: a receive port that is not a receive right is MACH_RCV_INVALID_NAME (0x10004002): an unknown name, MACH_PORT_NULL, MACH_PORT_DEAD, a send right (the task's own name) and a dead name; a call with no option at all does nothing and returns 0. A port that is a member of a port set can still be received from directly (MACH_RCV_TIMED_OUT here).
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
    u32 dead = 0;
    poly_mach6(MACH__kernelrpc_mach_port_allocate_trap, task, MACH_PORT_RIGHT_DEAD_NAME, (i64)&dead, 0, 0, 0);
    u32 opt = MACH_RCV_MSG | MACH_RCV_TIMEOUT;
    check("an unknown name: MACH_RCV_INVALID_NAME", poly_mach_msg(m, opt, 0, sizeof m, 0x12345, 0) == MACH_RCV_INVALID_NAME, 0);
    check("MACH_PORT_NULL: MACH_RCV_INVALID_NAME", poly_mach_msg(m, opt, 0, sizeof m, MACH_PORT_NULL, 0) == MACH_RCV_INVALID_NAME, 0);
    check("MACH_PORT_DEAD: MACH_RCV_INVALID_NAME", poly_mach_msg(m, opt, 0, sizeof m, MACH_PORT_DEAD, 0) == MACH_RCV_INVALID_NAME, 0);
    check("a send right (the task's own name): MACH_RCV_INVALID_NAME", poly_mach_msg(m, opt, 0, sizeof m, (u32)task, 0) == MACH_RCV_INVALID_NAME, 0);
    check("a dead name: MACH_RCV_INVALID_NAME", poly_mach_msg(m, opt, 0, sizeof m, dead, 0) == MACH_RCV_INVALID_NAME, 0);
    check("no option at all: returns 0", poly_mach_msg(m, 0, 0, 0, 0, 0) == 0, 0);
    u32 set = 0;
    poly_mach6(MACH__kernelrpc_mach_port_allocate_trap, task, MACH_PORT_RIGHT_PORT_SET, (i64)&set, 0, 0, 0);
    check("a port moved into a set (move_member trap 20)", (u32)poly_mach6(20, task, p, set, 0, 0, 0) == KERN_SUCCESS, 0);
    check("receiving from the member itself: MACH_RCV_TIMED_OUT", poly_mach_msg(m, opt, 0, sizeof m, p, 5) == MACH_RCV_TIMED_OUT, 0);
    check("receiving from the set: MACH_RCV_TIMED_OUT", poly_mach_msg(m, opt, 0, sizeof m, set, 5) == MACH_RCV_TIMED_OUT, 0);
    return finish();
}
#else
int test_main(void) { return finish(); }
#endif
