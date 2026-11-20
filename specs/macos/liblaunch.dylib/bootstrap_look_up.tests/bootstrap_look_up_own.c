/* poly-mac: libsystem
 * bootstrap_look_up of a name this process checked in: success, and what the caller gets is a send right to that port (the same
 * name as the receive right here, which becomes receive+send); a second look-up adds a user reference. */
#include "poly_mapi.h"
int test_main(void) {
    char name[100];
    service_name(name, sizeof name, "lu_own");
    mach_port_t recv = 0, send = 0, send2 = 0;
    check("check_in succeeds", bootstrap_check_in(bootstrap_port, name, &recv) == KERN_SUCCESS, 0);
    check("a receive right and nothing else", type_of(recv) == MACH_PORT_TYPE_RECEIVE, type_of(recv));
    check("look_up succeeds", bootstrap_look_up(bootstrap_port, name, &send) == KERN_SUCCESS, 0);
    check("the right found is a send right to the same port: the name is the receive right's", send == recv, send);
    check("the name now holds receive and send", type_of(recv) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND), type_of(recv));
    unsigned u1 = urefs_of(recv, MACH_PORT_RIGHT_SEND);
    check("a second look_up succeeds with the same name", bootstrap_look_up(bootstrap_port, name, &send2) == KERN_SUCCESS && send2 == send, send2);
    check("and adds one user reference", urefs_of(recv, MACH_PORT_RIGHT_SEND) == u1 + 1, urefs_of(recv, MACH_PORT_RIGHT_SEND));
    value("user references after the first look_up", u1);
    return finish();
}
