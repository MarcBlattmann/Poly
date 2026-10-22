/* syslog(2) as root, console actions: SYSLOG_ACTION_CONSOLE_OFF (6) and
 * SYSLOG_ACTION_CONSOLE_ON (7) return 0 (ON without a pending OFF too, and
 * twice in a row); SYSLOG_ACTION_CONSOLE_LEVEL (8) takes the level in len
 * and accepts 1..8 only: 0, 9, -1 and INT_MIN are EINVAL. bufp is ignored.
 * Only OFF/ON pairs are used, so the console level is left as found.
 * poly-diff: needs=root (only a privileged caller may change the console level)
 */
#include "poly_ipc.h"

#define SYS_syslog 103

int test_main(void) {
    i64 r = syscall3(SYS_syslog, 7, 0, 0);
    check("console_on_without_off_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 6, 0, 0);
    check("console_off_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 7, 0, 0);
    check("console_on_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 7, 0, 0);
    check("console_on_again_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 6, 8, -5);
    check("console_off_ignores_args", r == 0, r);
    r = syscall3(SYS_syslog, 7, 0xffff800000000000UL, 12345);
    check("console_on_ignores_args", r == 0, r);
    r = syscall3(SYS_syslog, 8, 0, 0);
    check("console_level_0_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 8, 0, 9);
    check("console_level_9_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 8, 0, -1);
    check("console_level_negative_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 8, 0, 0x80000000UL);
    check("console_level_int_min_einval", r == -EINVAL, r);
    return finish();
}
