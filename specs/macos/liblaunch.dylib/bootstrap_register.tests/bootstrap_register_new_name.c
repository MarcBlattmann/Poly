/* poly-mac: libsystem
 * bootstrap_register (deprecated, but it works for an unprivileged caller) of a new name with a port this process holds a send
 * right to: success; bootstrap_look_up then returns a send right to that port; registering the name again is
 * BOOTSTRAP_NAME_IN_USE (1101). */
#include "poly_mapi.h"
int test_main(void) {
    char name[100];
    service_name(name, sizeof name, "rg_new");
    mach_port_t port = new_port_with_send();
    kern_return_t kr = bootstrap_register(bootstrap_port, name, port);
    check("register of a new name succeeds", kr == KERN_SUCCESS, kr);
    mach_port_t found = 0;
    kr = bootstrap_look_up(bootstrap_port, name, &found);
    check("look_up finds it", kr == KERN_SUCCESS, kr);
    check("the right is a send right to the same port (same name here)", found == port && type_of(port) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), found);
    kr = bootstrap_register(bootstrap_port, name, port);
    check("registering it again is BOOTSTRAP_NAME_IN_USE (1101)", kr == 1101, kr);
    char other[100];
    service_name(other, sizeof other, "rg_new2");
    mach_port_t port2 = new_port_with_send();
    check("another name with another port works", bootstrap_register(bootstrap_port, other, port2) == KERN_SUCCESS, 0);
    check("the first name still gives the first port", bootstrap_look_up(bootstrap_port, name, &found) == KERN_SUCCESS && found == port, found);
    return finish();
}
