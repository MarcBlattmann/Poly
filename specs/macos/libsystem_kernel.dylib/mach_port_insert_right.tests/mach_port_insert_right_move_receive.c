/* poly-mac: libsystem
 * MACH_MSG_TYPE_MOVE_RECEIVE through mach_port_insert_right moves a receive right to the free name given (the port keeps its queue
 * limit); onto its own name nothing changes. It fails, and the right being moved is destroyed with it (the old name
 * turns into a dead name), when the port has a send right in the task (KERN_RIGHT_EXISTS, 21), and it is KERN_NAME_EXISTS (13)
 * for a name that is another port's. */
#include "poly_mapi.h"
static mach_port_t free_name(void) {
    mach_port_t n = new_receive();
    mach_port_destroy(mach_task_self(), n);
    return n;
}
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_limits_t l = {3};
    mach_port_set_attributes(t, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, MACH_PORT_LIMITS_INFO_COUNT);
    mach_port_set_context(t, p, 0x1234);
    mach_port_t to = free_name();
    kern_return_t kr = mach_port_insert_right(t, to, p, MACH_MSG_TYPE_MOVE_RECEIVE);
    check("MOVE_RECEIVE of a receive-only port to a free name succeeds", kr == KERN_SUCCESS, kr);
    check("the new name is the receive right, the old name is invalid", type_of(to) == MACH_PORT_TYPE_RECEIVE && is_gone(p), type_of(to));
    mach_port_limits_t l2;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    mach_port_get_attributes(t, to, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l2, &c);
    mach_port_context_t ctx = 0;
    mach_port_get_context(t, to, &ctx);
    check("the queue limit moved with it", l2.mpl_qlimit == 3, l2.mpl_qlimit);
    value("context after the move (0x1234 before)", (long)ctx);
    check("onto its own name: succeeds, nothing changes", mach_port_insert_right(t, to, to, MACH_MSG_TYPE_MOVE_RECEIVE) == KERN_SUCCESS && type_of(to) == MACH_PORT_TYPE_RECEIVE, 0);
    mach_port_t q = new_port_with_send();
    mach_port_t to2 = free_name();
    kr = mach_port_insert_right(t, to2, q, MACH_MSG_TYPE_MOVE_RECEIVE);
    check("a port with a send right in the task: KERN_RIGHT_EXISTS (21)", kr == KERN_RIGHT_EXISTS, kr);
    check("and the port is destroyed: its name is a dead name now", type_of(q) == MACH_PORT_TYPE_DEAD_NAME, type_of(q));
    check("the target name stays free", is_gone(to2), type_of(to2));
    mach_port_t r = new_receive(), other = new_receive();
    kr = mach_port_insert_right(t, other, r, MACH_MSG_TYPE_MOVE_RECEIVE);
    check("onto another port's name: KERN_NAME_EXISTS (13)", kr == KERN_NAME_EXISTS, kr);
    value("the moved port's old name after that", type_of(r));
    return finish();
}
