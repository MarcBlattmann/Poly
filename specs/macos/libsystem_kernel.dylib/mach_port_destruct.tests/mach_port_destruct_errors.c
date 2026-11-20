/* poly-mac: libsystem
 * What mach_port_destruct refuses: names that are not in the task are KERN_INVALID_NAME (15); a name that holds no receive
 * right (send only, dead name, port set) is KERN_INVALID_RIGHT (17) and is left as it was; an invalid task is MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    kern_return_t kr = mach_port_destruct(t, 0x4321, 0, 0);
    check("a name that does not exist: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    check("MACH_PORT_NULL: KERN_INVALID_NAME", mach_port_destruct(t, MACH_PORT_NULL, 0, 0) == KERN_INVALID_NAME, 0);
    check("MACH_PORT_DEAD: KERN_INVALID_NAME", mach_port_destruct(t, MACH_PORT_DEAD, 0, 0) == KERN_INVALID_NAME, 0);
    kr = mach_port_destruct(t, bootstrap_port, 0, 0);
    check("a send right only: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    check("which stays", type_of(bootstrap_port) == MACH_PORT_TYPE_SEND, type_of(bootstrap_port));
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    kr = mach_port_destruct(t, ps, 0, 0);
    check("a port set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    check("which stays", type_of(ps) == MACH_PORT_TYPE_PORT_SET, type_of(ps));
    mach_port_t d = new_port_with_send();
    mach_port_destruct(t, d, 0, 0);
    kr = mach_port_destruct(t, d, 0, 0);
    check("a dead name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_destruct(MACH_PORT_NULL, ps, 0, 0);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
