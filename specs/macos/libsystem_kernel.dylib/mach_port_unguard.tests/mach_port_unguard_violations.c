/* poly-mac: libsystem
 * mach_port_unguard with the wrong guard, on a port that is not guarded, or a second time ends the process with SIGKILL
 * (a guard violation); with the right guard it works. */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    if (is_child("child")) {
        mach_port_t p = new_receive();
        const char *w = child_arg();
        if (strcmp(w, "unguarded") == 0) mach_port_unguard(t, p, 0x1000);
        if (strcmp(w, "zero") == 0) mach_port_unguard(t, p, 0);
        if (strcmp(w, "wrong") == 0) {
            mach_port_guard(t, p, 0x1000, 0);
            mach_port_unguard(t, p, 0x2000);
        }
        if (strcmp(w, "twice") == 0) {
            mach_port_guard(t, p, 0x1000, 0);
            mach_port_unguard(t, p, 0x1000);
            mach_port_unguard(t, p, 0x1000);
        }
        if (strcmp(w, "right") == 0) {
            mach_port_guard(t, p, 0x1000, 0);
            return mach_port_unguard(t, p, 0x1000) == KERN_SUCCESS ? 0 : 3;
        }
        return 5;
    }
    check("the right guard: the child ends normally", wait_child(start_self("child", "right")) == 0, 0);
    check("a port that is not guarded: SIGKILL", wait_child(start_self("child", "unguarded")) == 1000 + SIGKILL, 0);
    check("guard 0 on a port that is not guarded: SIGKILL", wait_child(start_self("child", "zero")) == 1000 + SIGKILL, 0);
    check("a wrong guard: SIGKILL", wait_child(start_self("child", "wrong")) == 1000 + SIGKILL, 0);
    check("a second unguard: SIGKILL", wait_child(start_self("child", "twice")) == 1000 + SIGKILL, 0);
    return finish();
}
