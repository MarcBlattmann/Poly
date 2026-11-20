/* poly-mac: libsystem
 * bootstrap_register of a port the caller holds only a receive right to is accepted (a send right is made for the server), and
 * a look_up then gives a send right. When the receive right is destroyed the name goes. */
#include "poly_mapi.h"
int test_main(void) {
    char name[100];
    service_name(name, sizeof name, "rg_recv");
    mach_port_t port = new_receive();
    kern_return_t kr = bootstrap_register(bootstrap_port, name, port);
    value("register of a receive-only port", kr);
    mach_port_t found = 0;
    kr = bootstrap_look_up(bootstrap_port, name, &found);
    value("look_up of it", kr);
    check("a message sent to what was found arrives", kr == KERN_SUCCESS && send_empty(found, 9, 1000) == 0, kr);
    poly_rcv_t m;
    check("on the receive right", recv_msg(port, 1000, &m) == KERN_SUCCESS && m.h.msgh_id == 9, 0);
    destroy_port(port);
    check("after the receive right was destroyed the name is unknown", look_up_until(name, 1102, 5, &found) == 1102, 0);
    return finish();
}
