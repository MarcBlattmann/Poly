/* poly-mac: libsystem
 * bootstrap_look_up with a bootstrap port that is not a send right in the task: the send fails, MACH_SEND_INVALID_DEST (0x10000003). */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t p = 0x7777;
    check("MACH_PORT_NULL: MACH_SEND_INVALID_DEST", bootstrap_look_up(MACH_PORT_NULL, "com.poly.spec.x", &p) == MACH_SEND_INVALID_DEST, 0);
    check("a name that does not exist: MACH_SEND_INVALID_DEST", bootstrap_look_up(0x4321, "com.poly.spec.x", &p) == MACH_SEND_INVALID_DEST, 0);
    mach_port_t recv_only = new_receive();
    kern_return_t kr = bootstrap_look_up(recv_only, "com.poly.spec.x", &p);
    check("a receive right only: MACH_SEND_INVALID_DEST too", kr == MACH_SEND_INVALID_DEST, kr);
    mach_port_t dead = new_port_with_send();
    destroy_port(dead);
    check("a dead name: MACH_SEND_INVALID_DEST", bootstrap_look_up(dead, "com.poly.spec.x", &p) == MACH_SEND_INVALID_DEST, 0);
    check("the out port is left alone", p == 0x7777, p);
    return finish();
}
