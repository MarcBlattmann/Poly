/* setuid: as root (CAP_SETUID) it sets real, effective and saved uid
 * alike, so it cannot be undone; without the capability it only sets the
 * effective uid, and only to the real or saved uid (EPERM otherwise).
 * Checked in children.
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

int test_main(void) {
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        if (syscall1(SYS_setuid, 1234) != 0) code |= 1;
        unsigned r, e, s;
        syscall3(SYS_getresuid, &r, &e, &s);
        if (r != 1234 || e != 1234 || s != 1234) code |= 2;
        if (syscall1(SYS_setuid, 0) != -EPERM) code |= 4;
        if (syscall1(SYS_setuid, 1234) != 0) code |= 8;
        exit_group(code);
    }
    int code = wait_exit(pid);
    check("root_setuid_ok", code >= 0 && !(code & 1), code);
    check("root_setuid_sets_all_three", code >= 0 && !(code & 2), code);
    check("no_way_back_eperm", code >= 0 && !(code & 4), code);
    check("setuid_to_own_uid_ok", code >= 0 && !(code & 8), code);

    pid = fork();
    if (pid == 0) {
        int c = 0;
        /* Real 100, effective 200, saved 300; then no capability. */
        if (syscall3(SYS_setresuid, 100, 200, 300) != 0) c |= 1;
        if (syscall1(SYS_setuid, 300) != 0) c |= 2;
        unsigned r, e, s;
        syscall3(SYS_getresuid, &r, &e, &s);
        if (r != 100 || e != 300 || s != 300) c |= 4;
        if (syscall1(SYS_setuid, 100) != 0) c |= 8;
        syscall3(SYS_getresuid, &r, &e, &s);
        if (r != 100 || e != 100 || s != 300) c |= 16;
        if (syscall1(SYS_setuid, 200) != -EPERM) c |= 32;
        exit_group(c);
    }
    code = wait_exit(pid);
    check("setup_ok", code >= 0 && !(code & 1), code);
    check("unprivileged_to_saved_sets_euid_only", code >= 0 && !(code & 2) && !(code & 4), code);
    check("unprivileged_to_real_sets_euid_only", code >= 0 && !(code & 8) && !(code & 16), code);
    check("unprivileged_other_eperm", code >= 0 && !(code & 32), code);
    return finish();
}
