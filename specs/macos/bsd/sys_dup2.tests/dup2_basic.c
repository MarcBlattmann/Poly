/* dup2, BSD 90 (old fd, new fd): makes new fd refer to the same open file
 * as old fd (one offset) and returns new fd; a file that was open on new fd
 * is closed first; the copy does not have close-on-exec. */
#include "poly_mtest.h"

int test_main(void) {
    sysret a = bsd3(SYS_open, "a.txt", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, a.ret, "0123456789", 10);
    bsd3(SYS_lseek, a.ret, 0, SEEK_SET);
    sysret b = bsd3(SYS_open, "b.txt", O_CREAT | O_RDWR, 0644);
    sysret r = bsd2(SYS_dup2, a.ret, b.ret);
    check("dup2 returns the new descriptor", r.err == 0 && r.ret == b.ret, r.err ? r.err : r.ret);
    char buf[4];
    bsd3(SYS_read, a.ret, buf, 3);
    sysret n = bsd3(SYS_read, b.ret, buf, 3);
    check("the offset is shared: the copy continues at 3", n.ret == 3 && buf[0] == '3', n.ret);
    struct poly_stat64 sa, sb;
    bsd2(SYS_fstat64, a.ret, &sa);
    bsd2(SYS_fstat64, b.ret, &sb);
    check("the copy is the same file (inode)", sa.st_ino == sb.st_ino, 0);
    bsd3(SYS_fcntl, a.ret, F_SETFD, FD_CLOEXEC);
    bsd2(SYS_dup2, a.ret, 20);
    check("the new descriptor has no close-on-exec", bsd3(SYS_fcntl, 20, F_GETFD, 0).ret == 0, 0);
    sysret hi = bsd2(SYS_dup2, a.ret, 50);
    check("a descriptor far above the used ones: 50", hi.err == 0 && hi.ret == 50, hi.ret);
    return finish();
}
