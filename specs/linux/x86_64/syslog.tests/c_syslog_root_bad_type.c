/* syslog(2) as root: an unknown action (11, 12, 99, -1, INT_MAX, INT_MIN)
 * is EINVAL, whatever bufp and len are. The type is an int: bits above
 * the low 32 are ignored, so 0x100000000 + 11 is still EINVAL and
 * 0x100000000 + 1 is SYSLOG_ACTION_OPEN (0).
 * poly-diff: needs=root (an unprivileged caller gets EPERM instead)
 */
#include "poly_ipc.h"

#define SYS_syslog 103

int test_main(void) {
    char buf[32];
    i64 r = syscall3(SYS_syslog, 11, 0, 0);
    check("type_11_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 12, buf, sizeof buf);
    check("type_12_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 99, 0, 0);
    check("type_99_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, -1, buf, sizeof buf);
    check("type_negative_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 0x7fffffff, 0, 0);
    check("type_int_max_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 0x80000000UL, 0, 0);
    check("type_int_min_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 0x10000000bUL, 0, 0);
    check("type_high_bits_ignored_still_einval", r == -EINVAL, r);
    r = syscall3(SYS_syslog, 0x100000001UL, 0, 0);
    check("type_high_bits_ignored_open", r == 0, r);
    return finish();
}
