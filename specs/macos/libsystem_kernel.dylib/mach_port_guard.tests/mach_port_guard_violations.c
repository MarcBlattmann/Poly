/* poly-mac: libsystem
 * What a guarded receive right does not allow, each ending the process with SIGKILL (a guard violation): mach_port_destruct,
 * mach_port_destroy and mach_port_mod_refs(-1 of the receive right) without the guard. What it allows: send rights come and
 * go, notifications, set membership, mach_port_set_context (which changes the guard). */
#include "poly_mapi.h"
int test_main(void) {
    mach_port_t t = mach_task_self();
    if (is_child("child")) {
        mach_port_t p = new_port_with_send();
        mach_port_guard(t, p, 0x1000, 0);
        const char *w = child_arg();
        if (strcmp(w, "destroy") == 0) mach_port_destroy(t, p);
        if (strcmp(w, "mod_refs") == 0) mach_port_mod_refs(t, p, MACH_PORT_RIGHT_RECEIVE, -1);
        if (strcmp(w, "destruct") == 0) mach_port_destruct(t, p, 0, 0);
        if (strcmp(w, "allowed") == 0) {
            mach_port_t ps = 0, notify = new_receive(), prev = 0;
            mach_port_allocate(t, MACH_PORT_RIGHT_PORT_SET, &ps);
            int ok = mach_port_mod_refs(t, p, MACH_PORT_RIGHT_SEND, -1) == KERN_SUCCESS && mach_port_insert_right(t, p, p, MACH_MSG_TYPE_MAKE_SEND) == KERN_SUCCESS &&
                     mach_port_insert_member(t, p, ps) == KERN_SUCCESS && mach_port_extract_member(t, p, ps) == KERN_SUCCESS &&
                     mach_port_request_notification(t, p, MACH_NOTIFY_NO_SENDERS, 0, notify, MACH_MSG_TYPE_MAKE_SEND_ONCE, &prev) == KERN_SUCCESS;
            return ok ? 0 : 3;
        }
        return 5;
    }
    check("send rights, set membership and notifications are allowed", wait_child(start_self("child", "allowed")) == 0, 0);
    check("mach_port_destroy: SIGKILL", wait_child(start_self("child", "destroy")) == 1000 + SIGKILL, 0);
    check("mach_port_mod_refs -1 of the receive right: SIGKILL", wait_child(start_self("child", "mod_refs")) == 1000 + SIGKILL, 0);
    check("mach_port_destruct with guard 0: SIGKILL", wait_child(start_self("child", "destruct")) == 1000 + SIGKILL, 0);
    mach_port_t p = new_receive();
    mach_port_guard(t, p, 0x1000, 0);
    mach_port_set_context(t, p, 0x2000);
    mach_port_context_t ctx = 0;
    mach_port_get_context(t, p, &ctx);
    check("set_context on a guarded port changes the guard", ctx == 0x2000, (long)ctx);
    check("and the new value is the guard", mach_port_destruct(t, p, 0, 0x2000) == KERN_SUCCESS && is_gone(p), 0);
    return finish();
}
