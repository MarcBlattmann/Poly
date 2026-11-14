/* getdirentries64(2), BSD 344: EINVAL for a buffer of 32 bytes or for a
 * descriptor that is not a directory, EBADF, EFAULT for the buffer and for
 * the position pointer, which may not be NULL. */
#include "poly_mtest.h"

static void make_tree(void) {
    bsd2(SYS_mkdir, "d", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "d/a", O_CREAT | O_WRONLY, 0644).ret);
    bsd1(SYS_close, bsd3(SYS_open, "d/bb", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_mkdir, "d/sub", 0755);
}

static char buf[4096];

int test_main(void) {
    make_tree();
    i64 base;
    sysret d = bsd3(SYS_open, "d", O_RDONLY, 0);
    check_err("a 32-byte buffer: EINVAL", bsd4(SYS_getdirentries64, d.ret, buf, 32, &base), EINVAL);
    sysret f = bsd3(SYS_open, "d/a", O_RDONLY, 0);
    check_err("a file: EINVAL", bsd4(SYS_getdirentries64, f.ret, buf, sizeof buf, &base), EINVAL);
    check_err("fd 99: EBADF", bsd4(SYS_getdirentries64, 99, buf, sizeof buf, &base), EBADF);
    check_err("a NULL position pointer: EFAULT", bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, 0), EFAULT);
    bsd3(SYS_lseek, d.ret, 0, SEEK_SET);
    check_err("a buffer at address 8: EFAULT", bsd4(SYS_getdirentries64, d.ret, (void *)8, sizeof buf, &base), EFAULT);
    bsd3(SYS_lseek, d.ret, 0, SEEK_SET);
    check_err("a position pointer at address 8: EFAULT", bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, (void *)8), EFAULT);
    bsd3(SYS_lseek, d.ret, 0, SEEK_SET);
    sysret g = bsd4(SYS_getdirentries64, d.ret, buf, sizeof buf, &base);
    check("afterwards the directory reads normally", g.err == 0 && g.ret == 160, g.ret);
    return finish();
}
