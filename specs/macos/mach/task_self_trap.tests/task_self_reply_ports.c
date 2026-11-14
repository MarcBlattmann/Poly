/* task_self_trap, Mach trap 28, next to mach_reply_port (26): reply ports
 * get new names, never the task's, and the task's name stays. */
#include "poly_mtest.h"

int test_main(void) {
    i64 task = mach0(MACH_task_self_trap);
    i64 r1 = mach0(MACH_mach_reply_port), r2 = mach0(MACH_mach_reply_port);
    check("mach_reply_port gives a name", r1 != MACH_PORT_NULL, 0);
    check("a new one each time", r1 != r2, 0);
    check("never the task's", r1 != task && r2 != task, 0);
    check("the task's name stays", mach0(MACH_task_self_trap) == task, 0);
    return finish();
}
