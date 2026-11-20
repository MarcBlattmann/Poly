/* poly-mac: libsystem
 * What mach_port_extract_right refuses: names not in the task are KERN_INVALID_NAME (15), MACH_PORT_NULL KERN_INVALID_RIGHT (17),
 * a port set KERN_INVALID_RIGHT, a message type that is not a right to extract (99, COPY_RECEIVE) KERN_INVALID_VALUE (18),
 * MAKE_SEND_ONCE or MOVE_RECEIVE of a send-only name KERN_INVALID_RIGHT, an invalid task MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t ex = 0;
    mach_msg_type_name_t acq = 0;
    kern_return_t kr = mach_port_extract_right(t, 0x4321, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("a name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_extract_right(t, MACH_PORT_NULL, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("MACH_PORT_NULL: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    kr = mach_port_extract_right(t, ps, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("a port set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t p = new_port_with_send();
    kr = mach_port_extract_right(t, p, 99, &ex, &acq);
    check("message type 99: KERN_INVALID_VALUE", kr == KERN_INVALID_VALUE, kr);
    kr = mach_port_extract_right(t, p, MACH_MSG_TYPE_COPY_RECEIVE, &ex, &acq);
    check("COPY_RECEIVE: KERN_INVALID_VALUE", kr == KERN_INVALID_VALUE, kr);
    kr = mach_port_extract_right(t, bootstrap_port, MACH_MSG_TYPE_MAKE_SEND_ONCE, &ex, &acq);
    check("MAKE_SEND_ONCE of a send-only name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_extract_right(t, bootstrap_port, MACH_MSG_TYPE_MOVE_RECEIVE, &ex, &acq);
    check("MOVE_RECEIVE of a send-only name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_extract_right(MACH_PORT_NULL, p, MACH_MSG_TYPE_MAKE_SEND, &ex, &acq);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
