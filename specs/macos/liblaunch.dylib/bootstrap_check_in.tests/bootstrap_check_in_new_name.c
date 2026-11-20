/* poly-mac: libsystem
 * bootstrap_check_in of a name nobody has: success for any unprivileged caller, even when no job declares it; the result is a
 * new receive right (and only that). A second check_in of the same name by the same process is BOOTSTRAP_SERVICE_ACTIVE (1103). */
#include "poly_mapi.h"
int test_main(void) {
    char name[100];
    service_name(name, sizeof name, "ci_new");
    mach_port_t p = 0x7777, q = 0x7777;
    kern_return_t kr = bootstrap_check_in(bootstrap_port, name, &p);
    check("check_in of an unknown name succeeds", kr == KERN_SUCCESS, kr);
    check("a receive right", type_of(p) == MACH_PORT_TYPE_RECEIVE, type_of(p));
    mach_port_limits_t l;
    mach_msg_type_number_t c = MACH_PORT_LIMITS_INFO_COUNT;
    mach_port_get_attributes(mach_task_self(), p, MACH_PORT_LIMITS_INFO, (mach_port_info_t)&l, &c);
    check("with a queue limit of 2, not the default of 5", l.mpl_qlimit == 2, l.mpl_qlimit);
    kr = bootstrap_check_in(bootstrap_port, name, &q);
    check("a second check_in is BOOTSTRAP_SERVICE_ACTIVE (1103)", kr == 1103, kr);
    check("and leaves the out port alone", q == 0x7777, q);
    check("the first right is still good", type_of(p) == MACH_PORT_TYPE_RECEIVE, type_of(p));
    char odd[100];
    snprintf(odd, sizeof odd, "%s.with spaces/and..odd chars", name);
    check("any name is accepted, spaces and slashes too", bootstrap_check_in(bootstrap_port, odd, &q) == KERN_SUCCESS, 0);
    check("the empty name is accepted too", bootstrap_check_in(bootstrap_port, "", &q) == KERN_SUCCESS, 0);
    return finish();
}
