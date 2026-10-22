/* syslog(2) as root: SYSLOG_ACTION_CLOSE (0) and SYSLOG_ACTION_OPEN (1)
 * do nothing and return 0, whatever bufp and len are (NULL, unmapped or
 * kernel pointers, negative lengths); repeating them changes nothing.
 * poly-diff: needs=root (only a privileged caller may use these actions)
 */
#include "poly_ipc.h"

#define SYS_syslog 103

int test_main(void) {
    char buf[16];
    i64 r = syscall3(SYS_syslog, 1, 0, 0);
    check("open_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 0, 0, 0);
    check("close_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 1, 8, -1);
    check("open_ignores_bad_args", r == 0, r);
    r = syscall3(SYS_syslog, 0, 0xffff800000000000UL, 0x7fffffff);
    check("close_ignores_bad_args", r == 0, r);
    r = syscall3(SYS_syslog, 0, buf, sizeof buf);
    check("close_twice_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 1, buf, sizeof buf);
    check("open_twice_returns_0", r == 0, r);
    r = syscall3(SYS_syslog, 1, buf, sizeof buf);
    check("open_again_returns_0", r == 0, r);
    return finish();
}
