/* poly-mac: libsystem
 * Guards and mach_port_destruct: a port guarded with mach_port_guard needs that guard; a wrong one ends the process with
 * SIGKILL; a port that has a context but is not guarded takes guard 0. Unguarded, a port takes guard 0 again. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    if (is_child("child")) {
        mach_port_t p = new_receive();
        mach_port_guard(t, p, 0x1000, 0);
        if (strcmp(child_arg(), "wrong") == 0) mach_port_destruct(t, p, 0, 0x2000);
        if (strcmp(child_arg(), "none") == 0) mach_port_destruct(t, p, 0, 0);
        if (strcmp(child_arg(), "right") == 0) return mach_port_destruct(t, p, 0, 0x1000) == KERN_SUCCESS && is_gone(p) ? 0 : 3;
        return 5;
    }
    check("the right guard: succeeds", wait_child(start_self("child", "right")) == 0, 0);
    check("a wrong guard: SIGKILL", wait_child(start_self("child", "wrong")) == 1000 + SIGKILL, 0);
    check("no guard: SIGKILL", wait_child(start_self("child", "none")) == 1000 + SIGKILL, 0);
    mach_port_t c = new_receive();
    mach_port_set_context(t, c, 0x44);
    check("a context without a guard does not count", mach_port_destruct(t, c, 0, 0) == KERN_SUCCESS && is_gone(c), 0);
    mach_port_t u = new_receive();
    mach_port_guard(t, u, 0x1000, 0);
    mach_port_unguard(t, u, 0x1000);
    check("after unguard guard 0 works again", mach_port_destruct(t, u, 0, 0) == KERN_SUCCESS && is_gone(u), 0);
    return finish();
}
