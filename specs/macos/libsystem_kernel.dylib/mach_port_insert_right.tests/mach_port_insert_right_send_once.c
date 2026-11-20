/* poly-mac: libsystem
 * MACH_MSG_TYPE_MAKE_SEND_ONCE through mach_port_insert_right: a send-once right is a name of its own, so any free name takes
 * it (each call needs another one); under the port's own receive name it is KERN_NAME_EXISTS (13). MOVE_SEND_ONCE moves a
 * send-once right to another free name and the old name goes. Moving a name that is not a send-once right is KERN_INVALID_RIGHT. */
#include "poly_mapi.h"
static mach_port_t free_name(void) {
    mach_port_t n = new_receive();
    mach_port_destroy(mach_task_self(), n);
    return n;
}
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_t n1 = free_name();
    kern_return_t kr = mach_port_insert_right(t, n1, p, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    check("MAKE_SEND_ONCE under a free name succeeds", kr == KERN_SUCCESS, kr);
    check("the name is a send-once right", type_of(n1) == MACH_PORT_TYPE_SEND_ONCE, type_of(n1));
    check("the port is still only a receive right", type_of(p) == MACH_PORT_TYPE_RECEIVE, type_of(p));
    mach_port_t n2 = free_name();
    kr = mach_port_insert_right(t, n2, p, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    check("a second one under another name", kr == KERN_SUCCESS && type_of(n2) == MACH_PORT_TYPE_SEND_ONCE, kr);
    kr = mach_port_insert_right(t, p, p, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    check("under the receive right's own name: KERN_NAME_EXISTS (13)", kr == KERN_NAME_EXISTS, kr);
    mach_port_t n3 = free_name();
    kr = mach_port_insert_right(t, n3, n1, MACH_MSG_TYPE_MOVE_SEND_ONCE);
    check("MOVE_SEND_ONCE to another name succeeds", kr == KERN_SUCCESS, kr);
    check("the new name has it, the old name is gone", type_of(n3) == MACH_PORT_TYPE_SEND_ONCE && is_gone(n1), type_of(n3));
    mach_port_t n4 = free_name();
    kr = mach_port_insert_right(t, n4, p, MACH_MSG_TYPE_MOVE_SEND_ONCE);
    check("MOVE_SEND_ONCE of a receive right's name: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    check("and the free name stays free", is_gone(n4), type_of(n4));
    return finish();
}
