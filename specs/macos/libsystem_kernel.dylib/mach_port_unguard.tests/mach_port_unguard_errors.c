/* poly-mac: libsystem
 * mach_port_unguard on names that cannot be guarded: KERN_INVALID_NAME (15) for names not in the task, KERN_INVALID_RIGHT (17) for
 * a send right only, a port set or a dead name, MACH_SEND_INVALID_DEST for an invalid task. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    kern_return_t kr = mach_port_unguard(t, 0x4321, 1);
    check("a name that does not exist: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    check("MACH_PORT_NULL: KERN_INVALID_NAME", mach_port_unguard(t, MACH_PORT_NULL, 1) == KERN_INVALID_NAME, 0);
    kr = mach_port_unguard(t, bootstrap_port, 1);
    check("a send right only: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    check("a port set: KERN_INVALID_RIGHT", mach_port_unguard(t, ps, 1) == KERN_INVALID_RIGHT, 0);
    mach_port_t p = new_receive();
    kr = mach_port_unguard(MACH_PORT_NULL, p, 1);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
