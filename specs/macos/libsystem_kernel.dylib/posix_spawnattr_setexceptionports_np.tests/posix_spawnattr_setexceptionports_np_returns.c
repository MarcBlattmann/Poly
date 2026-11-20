/* poly-mac: libsystem
 * posix_spawnattr_setexceptionports_np(attr, mask, port, behavior, flavor) returns 0 whatever it is given: any mask (also 0), a
 * port that is MACH_PORT_NULL or does not exist, a behavior that is none (99). The port is not touched. */
#include "poly_mapi.h"
int test_main(void) {
    posix_spawnattr_t at;
    posix_spawnattr_init(&at);
    mach_port_t p = new_port_with_send();
    exception_mask_t m = EXC_MASK_BAD_ACCESS | EXC_MASK_ARITHMETIC;
    check("a normal call: 0", posix_spawnattr_setexceptionports_np(&at, m, p, EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES, THREAD_STATE_NONE) == 0, 0);
    check("EXC_MASK_ALL, EXCEPTION_STATE_IDENTITY: 0", posix_spawnattr_setexceptionports_np(&at, EXC_MASK_ALL, p, EXCEPTION_STATE_IDENTITY, THREAD_STATE_NONE) == 0, 0);
    check("mask 0: 0", posix_spawnattr_setexceptionports_np(&at, 0, p, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0, 0);
    check("behavior 99: 0", posix_spawnattr_setexceptionports_np(&at, m, p, 99, THREAD_STATE_NONE) == 0, 0);
    check("MACH_PORT_NULL: 0", posix_spawnattr_setexceptionports_np(&at, m, MACH_PORT_NULL, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0, 0);
    check("a name that does not exist: 0", posix_spawnattr_setexceptionports_np(&at, m, 0x4321, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0, 0);
    check("the port is untouched", type_of(p) == (MACH_PORT_TYPE_RECEIVE | MACH_PORT_TYPE_SEND) && urefs_of(p, MACH_PORT_RIGHT_SEND) == 1, type_of(p));
    posix_spawnattr_destroy(&at);
    return finish();
}
