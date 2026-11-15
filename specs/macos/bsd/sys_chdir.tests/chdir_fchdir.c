/* fchdir, BSD 13 (next to chdir): changes the working directory to an open
 * directory; ENOTDIR for the descriptor of a file, EBADF for one that is not
 * open. The open directory keeps working after the working directory
 * changes elsewhere. */
#include "poly_mtest.h"

int test_main(void) {
    struct poly_stat64 st;
    bsd2(SYS_mkdir, "dir", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "dir/inner", O_CREAT | O_WRONLY, 0644).ret);
    sysret d = bsd3(SYS_open, "dir", O_RDONLY, 0);
    sysret f = bsd3(SYS_open, "dir/inner", O_RDONLY, 0);
    check_ok("fchdir to the open directory", bsd1(SYS_fchdir, d.ret));
    check_ok("inner is visible", bsd2(SYS_stat64, "inner", &st));
    check_err("a file's descriptor: ENOTDIR", bsd1(SYS_fchdir, f.ret), ENOTDIR);
    check_err("a descriptor that is not open: EBADF", bsd1(SYS_fchdir, 99), EBADF);
    bsd1(SYS_chdir, "..");
    check_ok("back one level, the directory descriptor still works", bsd1(SYS_fchdir, d.ret));
    check_ok("inner is visible again", bsd2(SYS_stat64, "inner", &st));
    return finish();
}
