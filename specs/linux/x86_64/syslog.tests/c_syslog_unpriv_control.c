/* syslog(2) as an unprivileged process: SYSLOG_ACTION_CLEAR (5),
 * CONSOLE_OFF (6), CONSOLE_ON (7) and CONSOLE_LEVEL (8) need privilege:
 * EPERM, also for a console level outside 1..8 (EPERM before EINVAL). */
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
    check("unprivileged", become_unprivileged() != 0, 0);
    i64 r = syscall3(SYS_syslog, 5, 0, 0);
    check("clear_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 6, 0, 0);
    check("console_off_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 7, 0, 0);
    check("console_on_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 8, 0, 4);
    check("console_level_4_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 8, 0, 1);
    check("console_level_1_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 8, 0, 0);
    check("console_level_0_eperm_not_einval", r == -EPERM, r);
    r = syscall3(SYS_syslog, 8, 0, 9);
    check("console_level_9_eperm_not_einval", r == -EPERM, r);
    r = syscall3(SYS_syslog, 8, 0, -1);
    check("console_level_negative_eperm", r == -EPERM, r);
    return finish();
}
