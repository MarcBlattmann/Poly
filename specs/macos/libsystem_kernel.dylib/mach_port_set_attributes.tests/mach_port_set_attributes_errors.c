/* poly-mac: libsystem
 * What mach_port_set_attributes refuses: a count of 0 KERN_FAILURE (5); a flavor that cannot be set (999, MACH_PORT_RECEIVE_STATUS)
 * KERN_INVALID_ARGUMENT (4); names not in the task KERN_INVALID_NAME (15); a name without a receive right (send only, port set)
 * KERN_INVALID_RIGHT (17); an invalid task MACH_SEND_INVALID_DEST. A count of 2 is accepted for LIMITS_INFO. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_limits_t l = {3};
    kern_return_t kr = mach_port_set_attributes(t, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 0);
    check("a count of 0: KERN_FAILURE (5)", kr == KERN_FAILURE, kr);
    kr = mach_port_set_attributes(t, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 2);
    check("a count of 2 is accepted", kr == KERN_SUCCESS, kr);
    kr = mach_port_set_attributes(t, p, 999, (mach_port_info_t)&l, 1);
    check("flavor 999: KERN_INVALID_ARGUMENT (4)", kr == KERN_INVALID_ARGUMENT, kr);
    mach_port_status_t s;
    memset(&s, 0, sizeof s);
    kr = mach_port_set_attributes(t, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, MACH_PORT_RECEIVE_STATUS_COUNT);
    check("MACH_PORT_RECEIVE_STATUS cannot be set: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, kr);
    kr = mach_port_set_attributes(t, 0x4321, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 1);
    check("a name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    kr = mach_port_set_attributes(t, bootstrap_port, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 1);
    check("a send right only: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    kr = mach_port_set_attributes(t, ps, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 1);
    check("a port set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    kr = mach_port_set_attributes(MACH_PORT_NULL, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, 1);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
