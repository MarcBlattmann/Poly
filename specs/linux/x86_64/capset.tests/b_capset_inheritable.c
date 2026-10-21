/* capset of the inheritable set (in a child): with CAP_SETPCAP a
 * permitted capability can be added to the inheritable set and removed
 * again, and capget shows it; the version 1 interface (one record, low 32
 * capabilities) can drop a capability too, and it clears capabilities 32
 * and up from the effective and permitted sets; setting the same sets again is
 * allowed (a no-op).
 * poly-diff: syscalls=off (a child process)
 */
#include "poly_ipc.h"

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        struct cap_header h = { 0x20080522, 0 };
        struct cap_data d[2], e[2];
        syscall2(SYS_capget, &h, d);
        if (syscall2(SYS_capset, &h, d) != 0) code |= 1;
        d[0].inheritable = 1U << 1; /* CAP_DAC_OVERRIDE */
        if (syscall2(SYS_capset, &h, d) != 0) code |= 2;
        syscall2(SYS_capget, &h, e);
        if (e[0].inheritable != (1U << 1)) code |= 4;
        d[0].inheritable = 0;
        syscall2(SYS_capset, &h, d);
        syscall2(SYS_capget, &h, e);
        if (e[0].inheritable != 0) code |= 8;
        struct cap_header h1 = { 0x19980330, 0 };
        struct cap_data one;
        syscall2(SYS_capget, &h1, &one);
        one.effective &= ~(1U << 5); /* CAP_KILL */
        if (syscall2(SYS_capset, &h1, &one) != 0) code |= 16;
        syscall2(SYS_capget, &h, e);
        if (e[0].effective & (1U << 5)) code |= 32;
        if (e[1].effective != 0 || e[1].permitted != 0) code |= 64; /* high word cleared */
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("same_sets_noop_ok", code >= 0 && !(code & 1), code);
    check("add_inheritable_ok", code >= 0 && !(code & 2) && !(code & 4), code);
    check("remove_inheritable_ok", code >= 0 && !(code & 8), code);
    check("version_1_drop_ok", code >= 0 && !(code & 16) && !(code & 32), code);
    check("version_1_clears_high_capabilities", code >= 0 && !(code & 64), code);
    return finish();
}
