/* syslog(2) as an unprivileged process: SYSLOG_ACTION_READ (2) and
 * SYSLOG_ACTION_READ_CLEAR (4) always need privilege (CAP_SYSLOG or
 * CAP_SYS_ADMIN): EPERM at once, the read never blocks, nothing is written
 * to the buffer, and EPERM wins over the argument errors a privileged
 * caller would get (NULL buffer, negative length: EINVAL; bad pointer:
 * EFAULT). */
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
    char buf[128];
    check("unprivileged", become_unprivileged() != 0, 0);
    memset(buf, 0x5a, sizeof buf);
    i64 r = syscall3(SYS_syslog, 2, buf, sizeof buf);
    check("read_eperm", r == -EPERM, r);
    int untouched = 1;
    for (unsigned i = 0; i < sizeof buf; i++) untouched &= buf[i] == 0x5a;
    check("read_buffer_untouched", untouched, 0);
    r = syscall3(SYS_syslog, 2, 0, 16);
    check("read_null_buf_eperm_not_einval", r == -EPERM, r);
    r = syscall3(SYS_syslog, 2, buf, -1);
    check("read_negative_len_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 2, buf, 0);
    check("read_zero_len_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 2, 0xffff800000000000UL, 16);
    check("read_kernel_ptr_eperm_not_efault", r == -EPERM, r);
    r = syscall3(SYS_syslog, 4, buf, sizeof buf);
    check("read_clear_eperm", r == -EPERM, r);
    r = syscall3(SYS_syslog, 4, 0, -1);
    check("read_clear_bad_args_eperm", r == -EPERM, r);
    for (unsigned i = 0; i < sizeof buf; i++) untouched &= buf[i] == 0x5a;
    check("buffer_still_untouched", untouched, 0);
    return finish();
}
