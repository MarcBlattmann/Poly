/* poly-mac: libsystem
 * A special port given in the attributes of an unprivileged, unentitled caller does not reach the child: it starts with the
 * bootstrap port everyone has (it finds a name the parent checked in, and nothing is ever sent to the port given), and
 * posix_spawn itself succeeds. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t p = 0;
        kern_return_t kr = bootstrap_look_up(bootstrap_port, child_arg(), &p);
        check("child: look_up through its bootstrap port finds the parent's name", kr == KERN_SUCCESS, kr);
        mach_port_t sp = 0;
        task_get_special_port(mach_task_self(), TASK_BOOTSTRAP_PORT, &sp);
        check("child: task_get_special_port agrees with bootstrap_port", sp == bootstrap_port, sp);
        return finish();
    }
    char name[100];
    service_name(name, sizeof name, "sp_child");
    mach_port_t service = 0;
    (void)!bootstrap_check_in(bootstrap_port, name, &service);
    mach_port_t mine = new_port_with_send();
    posix_spawnattr_t at;
    posix_spawnattr_init(&at);
    posix_spawnattr_setspecialport_np(&at, mine, TASK_BOOTSTRAP_PORT);
    char *argv[4] = {poly_argv[0], "child", name, NULL};
    pid_t pid = 0;
    int err = posix_spawn(&pid, poly_argv[0], &at, NULL, argv, environ);
    check("posix_spawn succeeds", err == 0, err);
    int status = wait_child(pid);
    check("the child ends with status 0", status == 0, status);
    poly_rcv_t m;
    check("nothing was sent to the port given", recv_msg(mine, 100, &m) == MACH_RCV_TIMED_OUT, 0);
    posix_spawnattr_destroy(&at);
    return finish();
}
