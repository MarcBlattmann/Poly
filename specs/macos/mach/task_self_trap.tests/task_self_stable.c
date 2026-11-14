/* task_self_trap, Mach trap 28: the task's own port name, neither
 * MACH_PORT_NULL nor MACH_PORT_DEAD, a 32-bit name, the same every time. */
#include "poly_mtest.h"

int test_main(void) {
    i64 a = mach0(MACH_task_self_trap), b = mach0(MACH_task_self_trap);
    check("not MACH_PORT_NULL", a != MACH_PORT_NULL, 0);
    check("not MACH_PORT_DEAD", (u32)a != MACH_PORT_DEAD, 0);
    check("a 32-bit name", a == (i64)(u32)a, 0);
    check("the same name twice", a == b, 0);
    return finish();
}
