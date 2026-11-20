/* poly-mac: libsystem
 * What mach_port_insert_right refuses: a port name that is not in the task is KERN_INVALID_NAME (15) (MACH_PORT_NULL and
 * MACH_PORT_DEAD too); a message type that is no right (0, 99) or COPY_RECEIVE is KERN_INVALID_RIGHT (17); COPY_SEND of a name
 * with no send right is KERN_INVALID_RIGHT, of a dead name KERN_INVALID_CAPABILITY (20); an invalid task is MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_port_with_send();
    kern_return_t kr = mach_port_insert_right(t, p, 0x4321, MACH_MSG_TYPE_COPY_SEND);
    check("a port that is not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    check("MACH_PORT_NULL as the port: KERN_INVALID_NAME", mach_port_insert_right(t, p, MACH_PORT_NULL, MACH_MSG_TYPE_COPY_SEND) == KERN_INVALID_NAME, 0);
    check("MACH_PORT_DEAD as the port: KERN_INVALID_NAME", mach_port_insert_right(t, p, MACH_PORT_DEAD, MACH_MSG_TYPE_COPY_SEND) == KERN_INVALID_NAME, 0);
    kr = mach_port_insert_right(t, p, p, 0);
    check("message type 0: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_insert_right(t, p, p, 99);
    check("message type 99: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_insert_right(t, p, p, MACH_MSG_TYPE_COPY_RECEIVE);
    check("COPY_RECEIVE: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t r = new_receive();
    kr = mach_port_insert_right(t, r, r, MACH_MSG_TYPE_COPY_SEND);
    check("COPY_SEND of a name with no send right: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t d = new_port_with_send();
    mach_port_destruct(t, d, 0, 0);
    kr = mach_port_insert_right(t, p, d, MACH_MSG_TYPE_COPY_SEND);
    check("COPY_SEND of a dead name: KERN_INVALID_CAPABILITY (20)", kr == KERN_INVALID_CAPABILITY, kr);
    kr = mach_port_insert_right(MACH_PORT_NULL, p, p, MACH_MSG_TYPE_COPY_SEND);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
