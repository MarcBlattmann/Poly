/* ioctl, BSD 54, descriptor requests: FIOCLEX (0x20006601) and FIONCLEX
 * (0x20006602) set and clear close-on-exec (seen with fcntl F_GETFD),
 * FIONBIO (0x8004667e, int argument) sets and clears O_NONBLOCK (F_GETFL);
 * a request the object does not know is ENOTTY. */
#include "poly_mtest.h"

int test_main(void) {
    sysret p = bsd0(SYS_pipe);
    check("a new pipe end has no close-on-exec", bsd3(SYS_fcntl, p.ret, F_GETFD, 0).ret == 0, 0);
    check_ok("FIOCLEX", bsd3(SYS_ioctl, p.ret, FIOCLEX, 0));
    check("close-on-exec is set", bsd3(SYS_fcntl, p.ret, F_GETFD, 0).ret == FD_CLOEXEC, 0);
    check_ok("FIONCLEX", bsd3(SYS_ioctl, p.ret, FIONCLEX, 0));
    check("close-on-exec is clear", bsd3(SYS_fcntl, p.ret, F_GETFD, 0).ret == 0, 0);
    i32 on = 1, off = 0;
    check_ok("FIONBIO 1", bsd3(SYS_ioctl, p.ret, FIONBIO, &on));
    check("O_NONBLOCK is set", (bsd3(SYS_fcntl, p.ret, F_GETFL, 0).ret & O_NONBLOCK) != 0, 0);
    check_ok("FIONBIO 0", bsd3(SYS_ioctl, p.ret, FIONBIO, &off));
    check("O_NONBLOCK is clear", (bsd3(SYS_fcntl, p.ret, F_GETFL, 0).ret & O_NONBLOCK) == 0, 0);
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644);
    check_err("an unknown request on a pipe: ENOTTY", bsd3(SYS_ioctl, p.ret, 0x12345678, 0), ENOTTY);
    check_err("an unknown request on a file: ENOTTY", bsd3(SYS_ioctl, f.ret, 0x12345678, 0), ENOTTY);
    return finish();
}
