/* poly-mac: libsystem
 * What mach_port_insert_member refuses: names not in the task KERN_INVALID_NAME (15); the member not a receive right (a port
 * set, a send right only, MACH_PORT_NULL) and the target not a port set (a port, MACH_PORT_NULL) KERN_INVALID_RIGHT (17); an
 * invalid task MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self(), ps = 0, ps2 = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps2);
    mach_port_t p = new_receive(), q = new_receive();
    kern_return_t kr = mach_port_insert_member(t, 0x4321, ps);
    check("a member name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_insert_member(t, p, 0x4321);
    check("a set name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_insert_member(t, ps2, ps);
    check("a port set as the member: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_insert_member(t, bootstrap_port, ps);
    check("a send right only as the member: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_insert_member(t, p, q);
    check("a port as the set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    check("MACH_PORT_NULL as the member: KERN_INVALID_RIGHT", mach_port_insert_member(t, MACH_PORT_NULL, ps) == KERN_INVALID_RIGHT, 0);
    check("MACH_PORT_NULL as the set: KERN_INVALID_RIGHT", mach_port_insert_member(t, p, MACH_PORT_NULL) == KERN_INVALID_RIGHT, 0);
    kr = mach_port_insert_member(MACH_PORT_NULL, p, ps);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    mach_port_t pws = new_port_with_send();
    check("a port with a send right too is accepted", mach_port_insert_member(t, pws, ps) == KERN_SUCCESS, 0);
    return finish();
}
