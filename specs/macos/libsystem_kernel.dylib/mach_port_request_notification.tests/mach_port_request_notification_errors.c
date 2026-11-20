/* poly-mac: libsystem
 * What mach_port_request_notification refuses: a name not in the task or a notify name not in the task KERN_INVALID_NAME (15);
 * an unknown notification id or MACH_NOTIFY_SEND_ONCE KERN_INVALID_VALUE (18); MAKE_SEND, COPY_SEND, MOVE_SEND as the
 * notify right's type answer -300 (MIG client type check failure); the call still takes a MOVE_SEND
 * right off the notify name; a send-only name for NO_SENDERS KERN_INVALID_RIGHT (17); an invalid task MACH_SEND_INVALID_DEST; a NULL previous pointer
 * KERN_MEMORY_ERROR (10). */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self(), prev = 0;
    mach_port_t r = new_receive(), nt = new_port_with_send();
    kern_return_t kr = mach_port_request_notification(t, 0x4321, MACH_NOTIFY_DEAD_NAME, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("a name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_NO_SENDERS, 0, 0x4321, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("a notify name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_request_notification(t, r, 999, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("an unknown notification id: KERN_INVALID_VALUE", kr == KERN_INVALID_VALUE, kr);
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_SEND_ONCE, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("MACH_NOTIFY_SEND_ONCE cannot be requested: KERN_INVALID_VALUE", kr == KERN_INVALID_VALUE, kr);
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_MAKE_SEND, &prev);
    check("MAKE_SEND as the right's type: -300", kr == -300, kr);
    check("MAKE_SEND leaves the notify name as it was", type_of(nt) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), type_of(nt));
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_COPY_SEND, &prev);
    check("COPY_SEND: -300", kr == -300, kr);
    check("COPY_SEND leaves it as it was", type_of(nt) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), type_of(nt));
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_MOVE_SEND, &prev);
    check("MOVE_SEND: -300", kr == -300, kr);
    check("but MOVE_SEND takes the send right away: the name is a receive right only", type_of(nt) == MACH_PORT_TYPE_RECEIVE, type_of(nt));
    kr = mach_port_request_notification(t, bootstrap_port, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("NO_SENDERS on a send-only name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_request_notification(MACH_PORT_NULL, r, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    kr = mach_port_request_notification(t, r, MACH_NOTIFY_NO_SENDERS, 0, nt, MACH_MSG_TYPE_MAKE_SEND_ONCE, NULL);
    check("a NULL previous pointer: KERN_MEMORY_ERROR (10)", kr == 10, kr);
    return finish();
}
