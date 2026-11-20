/* poly-mac: libsystem
 * What bootstrap_register refuses: a name another process holds is BOOTSTRAP_NOT_PRIVILEGED (1100), a name this process holds
 * (registered or checked in) is BOOTSTRAP_NAME_IN_USE (1101), a port name that is no right is KERN_INVALID_RIGHT (17), a
 * bootstrap port that is no send right is MACH_SEND_INVALID_DEST. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t port = new_port_with_send();
        kern_return_t kr = bootstrap_register(bootstrap_port, child_arg(), port);
        check("child: a name the parent holds is 1100", kr == 1100, kr);
        return finish();
    }
    char name[100], ci[100];
    service_name(name, sizeof name, "rg_ref");
    service_name(ci, sizeof ci, "rg_ref_ci");
    mach_port_t port = new_port_with_send(), recv = 0;
    check("parent registers", bootstrap_register(bootstrap_port, name, port) == KERN_SUCCESS, 0);
    pid_t pid = start_self("child", name);
    check("the child ends with status 0", wait_child(pid) == 0, 0);
    (void)!bootstrap_check_in(bootstrap_port, ci, &recv);
    kern_return_t kr = bootstrap_register(bootstrap_port, ci, port);
    check("a name checked in by this process is 1101", kr == 1101, kr);
    char fresh[100];
    service_name(fresh, sizeof fresh, "rg_ref_fresh");
    kr = bootstrap_register(bootstrap_port, fresh, 0x4321);
    check("a port name that does not exist is KERN_INVALID_RIGHT", kr == KERN_INVALID_RIGHT, kr);
    mach_port_t p = 0;
    check("and the name stays free", bootstrap_look_up(bootstrap_port, fresh, &p) == 1102, 0);
    kr = bootstrap_register(MACH_PORT_NULL, fresh, port);
    check("a null bootstrap port is MACH_SEND_INVALID_DEST", kr == MACH_SEND_INVALID_DEST, kr);
    return finish();
}
