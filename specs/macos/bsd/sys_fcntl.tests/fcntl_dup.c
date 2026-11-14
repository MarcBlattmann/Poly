/* fcntl, BSD 92: F_DUPFD returns the lowest free descriptor that is at
 * least the argument (without close-on-exec), F_DUPFD_CLOEXEC (67) the same
 * with close-on-exec set; the copies share the file. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd3(SYS_open, "f.txt", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, a.ret, "hello", 5);
    sysret d = bsd3(SYS_fcntl, a.ret, F_DUPFD, 20);
    check("F_DUPFD 20: the lowest free from 20", d.err == 0 && d.ret == 20, d.ret);
    sysret d2 = bsd3(SYS_fcntl, a.ret, F_DUPFD, 20);
    check("again: 21", d2.err == 0 && d2.ret == 21, d2.ret);
    check("the copy has no close-on-exec", bsd3(SYS_fcntl, 20, F_GETFD, 0).ret == 0, 0);
    sysret c = bsd3(SYS_fcntl, a.ret, F_DUPFD_CLOEXEC, 30);
    check("F_DUPFD_CLOEXEC 30: 30", c.err == 0 && c.ret == 30, c.ret);
    check("with close-on-exec", bsd3(SYS_fcntl, 30, F_GETFD, 0).ret == FD_CLOEXEC, 0);
    sysret low = bsd3(SYS_fcntl, a.ret, F_DUPFD, 0);
    check("F_DUPFD 0: the lowest free (4)", low.err == 0 && low.ret == a.ret + 1, low.ret);
    char b[8];
    bsd3(SYS_lseek, 20, 0, SEEK_SET);
    sysret n = bsd3(SYS_read, 30, b, 5);
    check("a copy shares the offset", n.ret == 5 && mem_eq(b, "hello", 5), n.ret);
    return finish();
}
