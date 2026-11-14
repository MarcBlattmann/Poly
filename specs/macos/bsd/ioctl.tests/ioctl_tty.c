/* ioctl, BSD 54 (fd, request, argument): TIOCGWINSZ (0x40087468, window
 * size) is not for a file or pipe (ENOTTY), not for /dev/null (ENODEV), and
 * EBADF for a descriptor that is not open: the first thing a program does
 * to find out whether it writes to a terminal. */
#include "poly_mtest.h"

int test_main(void) {
    char ws[8];
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644);
    sysret n = bsd3(SYS_open, "/dev/null", O_RDWR, 0);
    sysret p = bsd0(SYS_pipe);
    check_err("a regular file: ENOTTY", bsd3(SYS_ioctl, f.ret, TIOCGWINSZ, ws), ENOTTY);
    check_err("a pipe: ENOTTY", bsd3(SYS_ioctl, p.ret, TIOCGWINSZ, ws), ENOTTY);
    check_err("/dev/null: ENODEV", bsd3(SYS_ioctl, n.ret, TIOCGWINSZ, ws), ENODEV);
    check_err("a directory: ENOTTY", bsd3(SYS_ioctl, bsd3(SYS_open, ".", O_RDONLY, 0).ret, TIOCGWINSZ, ws), ENOTTY);
    check_err("a descriptor that is not open: EBADF", bsd3(SYS_ioctl, 99, TIOCGWINSZ, ws), EBADF);
    return finish();
}
