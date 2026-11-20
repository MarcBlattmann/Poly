/* poly-mac: libsystem
 * A name held by a process that has exited can be checked in again by another process, and one whose receive right was
 * destroyed can be checked in again by the same process; the new port is a different port. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t p = 0;
        if (bootstrap_check_in(bootstrap_port, child_arg(), &p)) return 2;
        usleep(1200000);
        return 0;
    }
    char name[100];
    service_name(name, sizeof name, "ci_again");
    pid_t pid = start_self("child", name);
    mach_port_t p = 0;
    look_up_until(name, KERN_SUCCESS, 5, &p);
    mach_port_t mine = 0;
    check("while the child holds it: BOOTSTRAP_NOT_PRIVILEGED (1100)", bootstrap_check_in(bootstrap_port, name, &mine) == 1100, 0);
    check("the child ends with status 0", wait_child(pid) == 0, 0);
    look_up_until(name, 1102, 5, &p);
    kern_return_t kr = bootstrap_check_in(bootstrap_port, name, &mine);
    check("then the parent can check it in", kr == KERN_SUCCESS, kr);
    destroy_port(mine);
    mach_port_t again = 0;
    look_up_until(name, 1102, 5, &p);
    kr = bootstrap_check_in(bootstrap_port, name, &again);
    check("and again after it destroyed its receive right", kr == KERN_SUCCESS, kr);
    return finish();
}
