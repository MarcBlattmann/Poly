/* poly-mac: libsystem
 * Every task_get_special_port(TASK_BOOTSTRAP_PORT) returns the same name and adds one user reference to the send right; it
 * can be given back with mach_port_deallocate. bootstrap_port itself keeps its right. */
#include "poly_mapi.h"
int test_main(void) {
    unsigned u0 = urefs_of(bootstrap_port, MACH_PORT_RIGHT_SEND);
    check("it holds at least one user reference", u0 >= 1, u0);
    mach_port_t a = 0, b = 0;
    task_get_special_port(mach_task_self(), TASK_BOOTSTRAP_PORT, &a);
    task_get_special_port(mach_task_self(), TASK_BOOTSTRAP_PORT, &b);
    check("two calls give the same name", a == b && a == bootstrap_port, a);
    check("and add two user references", urefs_of(bootstrap_port, MACH_PORT_RIGHT_SEND) == u0 + 2, urefs_of(bootstrap_port, MACH_PORT_RIGHT_SEND));
    mach_port_deallocate(mach_task_self(), a);
    mach_port_deallocate(mach_task_self(), b);
    check("giving them back restores the count", urefs_of(bootstrap_port, MACH_PORT_RIGHT_SEND) == u0, urefs_of(bootstrap_port, MACH_PORT_RIGHT_SEND));
    mach_port_t p;
    check("bootstrap_port still works", bootstrap_look_up(bootstrap_port, "com.poly.spec.nosuch", &p) == 1102, 0);
    return finish();
}
