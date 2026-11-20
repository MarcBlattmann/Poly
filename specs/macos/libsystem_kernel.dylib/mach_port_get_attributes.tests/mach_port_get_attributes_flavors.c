/* poly-mac: libsystem
 * The other flavors and the refusals of mach_port_get_attributes: MACH_PORT_LIMITS_INFO (count 1) gives the queue limit,
 * MACH_PORT_DNREQUESTS_SIZE (count 1) gives 0 for a new port; flavor 999 and MACH_PORT_TEMPOWNER are KERN_INVALID_ARGUMENT (4); a count too
 * small is KERN_FAILURE (5); names not in the task KERN_INVALID_NAME (15); a name without a receive right KERN_INVALID_RIGHT (17); an invalid
 * task MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    mach_port_t p = new_receive();
    mach_port_limits_t l;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    kern_return_t kr = mach_port_get_attributes(t, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    check("LIMITS_INFO: count 1, queue limit 5", kr == KERN_SUCCESS && c == 1 && l.mpl_qlimit == 5, l.mpl_qlimit);
    mach_port_name_t dn = 99;
    c = 1;
    kr = mach_port_get_attributes(t, p, MACH_PORT_DNREQUESTS_SIZE, (mach_port_info_t)&dn, &c);
    check("DNREQUESTS_SIZE: succeeds with 0", kr == KERN_SUCCESS && dn == 0, dn);
    unsigned v = 0;
    c = 1;
    kr = mach_port_get_attributes(t, p, 999, (mach_port_info_t)&v, &c);
    check("flavor 999: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, kr);
    c = 1;
    kr = mach_port_get_attributes(t, p, MACH_PORT_TEMPOWNER, (mach_port_info_t)&v, &c);
    check("MACH_PORT_TEMPOWNER: KERN_INVALID_ARGUMENT", kr == KERN_INVALID_ARGUMENT, kr);
    c = 0;
    kr = mach_port_get_attributes(t, p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    check("a count of 0: KERN_FAILURE (5)", kr == KERN_FAILURE, kr);
    mach_port_status_t s;
    c = 5;
    kr = mach_port_get_attributes(t, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    value("RECEIVE_STATUS with a count of 5", kr);
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    kr = mach_port_get_attributes(t, 0x4321, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("a name not in the task: KERN_INVALID_NAME", kr == KERN_INVALID_NAME, kr);
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    kr = mach_port_get_attributes(t, bootstrap_port, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("a send right only: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t ps = 0;
    mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    kr = mach_port_get_attributes(t, ps, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("a port set: KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    c = MACH_PORT_RECEIVE_STATUS_COUNT;
    kr = mach_port_get_attributes(MACH_PORT_NULL, p, MACH_PORT_RECEIVE_STATUS, (mach_port_info_t)&s, &c);
    check("an invalid task: MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
