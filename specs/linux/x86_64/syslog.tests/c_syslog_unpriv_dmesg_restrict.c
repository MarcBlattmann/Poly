/* syslog(2) as an unprivileged process: SYSLOG_ACTION_READ_ALL (3) and
 * SYSLOG_ACTION_SIZE_BUFFER (10) are the two actions an unprivileged
 * process may use, but only while /proc/sys/kernel/dmesg_restrict is 0;
 * when it is 1 they are EPERM like everything else. With 0, READ_ALL with a
 * NULL buffer is EINVAL, with length 0 returns 0, otherwise returns at
 * most len bytes; SIZE_BUFFER returns the (positive) buffer size. Where the
 * file cannot be read, the two actions at least agree. */
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

/* 0 or 1 from /proc/sys/kernel/dmesg_restrict, -1 if unreadable. */
static int dmesg_restrict(void) {
    i64 fd = syscall2(SYS_open, "/proc/sys/kernel/dmesg_restrict", 0);
    if (fd < 0) return -1;
    char c = 0;
    i64 n = syscall3(SYS_read, fd, &c, 1);
    syscall1(SYS_close, fd);
    if (n != 1) return -1;
    return c == '0' ? 0 : c == '1' ? 1 : -1;
}

int test_main(void) {
    static char buf[4096];
    check("unprivileged", become_unprivileged() != 0, 0);
    int dr = dmesg_restrict();
    i64 r3 = syscall3(SYS_syslog, 3, buf, sizeof buf);
    i64 r10 = syscall3(SYS_syslog, 10, 0, 0);
    i64 rnull = syscall3(SYS_syslog, 3, 0, 16);
    i64 rzero = syscall3(SYS_syslog, 3, buf, 0);
    if (dr == 1) {
        check("read_all_follows_dmesg_restrict", r3 == -EPERM, r3);
        check("size_buffer_follows_dmesg_restrict", r10 == -EPERM, r10);
        check("read_all_null_follows_dmesg_restrict", rnull == -EPERM, rnull);
        check("read_all_zero_len_follows_dmesg_restrict", rzero == -EPERM, rzero);
    } else if (dr == 0) {
        check("read_all_follows_dmesg_restrict", r3 >= 0 && r3 <= (i64)sizeof buf, r3);
        check("size_buffer_follows_dmesg_restrict", r10 > 0, r10);
        check("read_all_null_follows_dmesg_restrict", rnull == -EINVAL, rnull);
        check("read_all_zero_len_follows_dmesg_restrict", rzero == 0, rzero);
    } else {
        check("read_all_follows_dmesg_restrict", r3 == -EPERM || r3 >= 0, r3);
        check("size_buffer_follows_dmesg_restrict", (r3 == -EPERM) == (r10 == -EPERM), r10);
        check("read_all_null_follows_dmesg_restrict", rnull == -EPERM || rnull == -EINVAL, rnull);
        check("read_all_zero_len_follows_dmesg_restrict", rzero == -EPERM || rzero == 0, rzero);
    }
    i64 r = syscall3(SYS_syslog, 5, 0, 0);
    check("clear_still_eperm", r == -EPERM, r);
    return finish();
}
