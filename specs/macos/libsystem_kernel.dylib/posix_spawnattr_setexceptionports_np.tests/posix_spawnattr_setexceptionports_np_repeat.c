/* poly-mac: libsystem
 * Several calls on one attribute set, with different masks and ports, all return 0 and the child still starts and runs normally;
 * an attribute set that was destroyed and made again does too. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        check("child: runs", 1, 0);
        return finish();
    }
    mach_port_t a = new_port_with_send(), b = new_port_with_send();
    posix_spawnattr_t at;
    posix_spawnattr_init(&at);
    int ok = posix_spawnattr_setexceptionports_np(&at, EXC_MASK_BAD_ACCESS, a, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0 &&
             posix_spawnattr_setexceptionports_np(&at, EXC_MASK_CRASH, b, EXCEPTION_STATE, THREAD_STATE_NONE) == 0 &&
             posix_spawnattr_setexceptionports_np(&at, EXC_MASK_BAD_ACCESS, b, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0;
    check("three calls all return 0", ok, 0);
    char *argv[3] = {poly_argv[0], "child", NULL};
    pid_t pid = 0;
    check("posix_spawn succeeds", posix_spawn(&pid, poly_argv[0], &at, NULL, argv, environ) == 0, 0);
    check("the child ends with status 0", wait_child(pid) == 0, 0);
    posix_spawnattr_destroy(&at);
    posix_spawnattr_init(&at);
    check("a new attribute set after destroy: set returns 0", posix_spawnattr_setexceptionports_np(&at, EXC_MASK_BAD_ACCESS, a, EXCEPTION_DEFAULT, THREAD_STATE_NONE) == 0, 0);
    posix_spawnattr_destroy(&at);
    return finish();
}
