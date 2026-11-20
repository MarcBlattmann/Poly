/* poly-mac: libsystem
 * Setting the bootstrap special port to MACH_PORT_NULL or to a name that does not exist does not stop the child from
 * starting, and the child's bootstrap port is the usual one too. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        check("child: bootstrap_port is a send right", type_of(bootstrap_port) == MACH_PORT_TYPE_SEND, type_of(bootstrap_port));
        return finish();
    }
    for (int i = 0; i < 2; i++) {
        posix_spawnattr_t at;
        posix_spawnattr_init(&at);
        posix_spawnattr_setspecialport_np(&at, i ? (mach_port_t)0x4321 : MACH_PORT_NULL, TASK_BOOTSTRAP_PORT);
        char *argv[3] = {poly_argv[0], "child", NULL};
        pid_t pid = 0;
        int err = posix_spawn(&pid, poly_argv[0], &at, NULL, argv, environ);
        check(i ? "a name that does not exist: posix_spawn succeeds" : "MACH_PORT_NULL: posix_spawn succeeds", err == 0, err);
        check("the child ends with status 0", wait_child(pid) == 0, 0);
        posix_spawnattr_destroy(&at);
    }
    return finish();
}
