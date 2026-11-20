/* poly-mac: libsystem
 * A child started with posix_spawn has a bootstrap_port of its own that works the same way: a send right, and a name the
 * parent checked in is found through it. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        check("child: bootstrap_port is a send right", type_of(bootstrap_port) == MACH_PORT_TYPE_SEND, type_of(bootstrap_port));
        mach_port_t p = 0;
        kern_return_t kr = bootstrap_look_up(bootstrap_port, child_arg(), &p);
        check("child: finds the parent's name", kr == KERN_SUCCESS, kr);
        mach_port_t q = 0;
        check("child: and not one nobody has", bootstrap_look_up(bootstrap_port, "com.poly.spec.nosuch", &q) == 1102, 0);
        return finish();
    }
    char name[100];
    service_name(name, sizeof name, "bp_inh");
    mach_port_t recv = 0;
    (void)!bootstrap_check_in(bootstrap_port, name, &recv);
    pid_t pid = start_self("child", name);
    int status = wait_child(pid);
    check("the child ends with status 0", status == 0, status);
    return finish();
}
