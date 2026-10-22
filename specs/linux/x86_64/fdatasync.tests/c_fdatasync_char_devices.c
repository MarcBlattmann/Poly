/* fdatasync on character devices: EINVAL (/dev/null, /dev/zero,
 * /dev/urandom), whatever the access mode. */
#include "a_sys.h"

int test_main(void) {
    i64 fd = a_open("/dev/null", O_RDWR, 0);
    expect("null_rdwr", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = a_open("/dev/null", O_RDONLY, 0);
    expect("null_rdonly", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = a_open("/dev/zero", O_RDONLY, 0);
    expect("zero", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    fd = a_open("/dev/urandom", O_RDONLY, 0);
    expect("urandom", syscall1(SYS_fdatasync, fd), -EINVAL);
    a_close(fd);
    return finish();
}
