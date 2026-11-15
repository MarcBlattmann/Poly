/* read(2), BSD 3: EBADF for descriptors not open for reading, EISDIR for a
 * directory, EFAULT for a buffer that cannot be written, but not at the end
 * of the file, where nothing is copied. */
#include "poly_mtest.h"

int test_main(void) {
    char buf[8];
    check_err("fd 77: EBADF", bsd3(SYS_read, 77, buf, 1), EBADF);
    sysret w = bsd3(SYS_open, "e.txt", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, w.ret, "0123456789", 10);
    check_err("a write-only descriptor: EBADF", bsd3(SYS_read, w.ret, buf, 1), EBADF);
    sysret d = bsd3(SYS_open, ".", O_RDONLY, 0);
    check_err("a directory: EISDIR", bsd3(SYS_read, d.ret, buf, 1), EISDIR);
    sysret r = bsd3(SYS_open, "e.txt", O_RDONLY, 0);
    check_err("a buffer at address 8: EFAULT", bsd3(SYS_read, r.ret, (void *)8, 4), EFAULT);
    sysret n = bsd3(SYS_read, r.ret, buf, 4);
    check("the failed read consumed nothing", n.ret == 4 && mem_eq(buf, "0123", 4), n.ret);
    bsd3(SYS_lseek, r.ret, 0, SEEK_END);
    n = bsd3(SYS_read, r.ret, (void *)8, 4);
    check("at the end, the same buffer: 0", n.err == 0 && n.ret == 0, n.err);
    return finish();
}
