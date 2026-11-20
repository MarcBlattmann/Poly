/* poly-mac: libsystem
 * A checked-in name belongs to the process that holds its receive right: when that process exits the name is unknown again,
 * and when the receive right is destroyed in the same process it is too. */
#include "poly_mapi.h"
int test_main(void) {
    if (is_child("child")) {
        mach_port_t p = 0;
        kern_return_t kr = bootstrap_check_in(bootstrap_port, child_arg(), &p);
        if (kr) return 2;
        usleep(1500000);
        return 0;
    }
    char name[100], name2[100];
    service_name(name, sizeof name, "lu_die");
    pid_t pid = start_self("child", name);
    mach_port_t p = 0;
    kern_return_t kr = look_up_until(name, KERN_SUCCESS, 5, &p);
    check("while the child lives, the name is found", kr == KERN_SUCCESS, kr);
    check("the child exits with status 0", wait_child(pid) == 0, 0);
    check("shortly after it exited the name is unknown", look_up_until(name, 1102, 5, &p) == 1102, 0);
    mach_port_t recv = 0;
    service_name(name2, sizeof name2, "lu_die2");
    (void)!bootstrap_check_in(bootstrap_port, name2, &recv);
    check("a checked-in name is found", bootstrap_look_up(bootstrap_port, name2, &p) == KERN_SUCCESS, 0);
    destroy_port(recv);
    check("after the receive right is destroyed it is unknown", look_up_until(name2, 1102, 5, &p) == 1102, 0);
    return finish();
}
