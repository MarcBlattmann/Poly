/* poly-mac: libsystem
 * mach_port_set_attributes(task, receive name, MACH_PORT_LIMITS_INFO, &{qlimit}, 1) sets the queue limit (0 to 1024); read back
 * with MACH_PORT_LIMITS_INFO and in mps_qlimit of the receive status. Limit 1025 is KERN_INVALID_VALUE (18) and changes nothing. */
#include "poly_mapi.h"
static int set_q(mach_port_t p, unsigned q) {
    mach_port_limits_t l = {q};
    return mach_port_set_attributes(mach_task_self(), p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, MACH_PORT_LIMITS_INFO_COUNT);
}
static unsigned get_q(mach_port_t p) {
    mach_port_limits_t l;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    mach_port_get_attributes(mach_task_self(), p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    return l.mpl_qlimit;
}
int test_main(void) {
    mach_port_t p = new_receive();
    check("the default is 5", get_q(p) == 5, get_q(p));
    check("set 2 succeeds", set_q(p, 2) == KERN_SUCCESS, 0);
    check("read back 2", get_q(p) == 2, get_q(p));
    mach_port_status_t s;
    mach_msg_type_number_t c = MACH_PORT_RECEIVE_STATUS_COUNT;
    mach_port_get_attributes(mach_task_self(), p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("and in the receive status", s.mps_qlimit == 2, s.mps_qlimit);
    check("set 1024 succeeds", set_q(p, 1024) == KERN_SUCCESS && get_q(p) == 1024, get_q(p));
    kern_return_t kr = set_q(p, 1025);
    check("set 1025 is KERN_INVALID_VALUE (18)", kr == KERN_INVALID_VALUE, kr);
    check("and the limit stays 1024", get_q(p) == 1024, get_q(p));
    check("set 0 succeeds", set_q(p, 0) == KERN_SUCCESS, 0);
    check("read back 0", get_q(p) == 0, get_q(p));
    return finish();
}
