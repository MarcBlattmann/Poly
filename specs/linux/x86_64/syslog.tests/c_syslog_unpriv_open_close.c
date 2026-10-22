/* syslog(2) as an unprivileged process: SYSLOG_ACTION_CLOSE (0) and
 * SYSLOG_ACTION_OPEN (1) are no-ops for a privileged caller, but an
 * unprivileged one gets EPERM for them, whatever bufp and len are (the
 * permission check comes before anything else). */
#include "poly_ipc.h"

#define SYS_syslog 103

/* Run as an unprivileged user: root becomes nobody (65534) for good, which
 * also empties its capability sets. Returns the effective uid afterwards. */
static i64 become_unprivileged(void) {
    if (syscall0(SYS_geteuid) == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
    }
    return syscall0(SYS_geteuid);
}

int test_main(void) {
    char buf[64];
    check("unprivileged", become_unprivileged() != 0, 0);
    i64 r = syscall3(SYS_syslog, 0, buf, sizeof buf);
    check("close_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 1, buf, sizeof buf);
    check("open_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 0, 0, 0);
    check("close_null_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 1, 8, -1);
    check("open_bad_args_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 1, buf, sizeof buf);
    check("open_still_eperm", r == -EPERM, r);
    return finish();
}
