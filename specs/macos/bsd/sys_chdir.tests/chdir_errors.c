/* chdir, BSD 12: ENOENT for a missing directory and the empty path, ENOTDIR
 * for a file and for a file used as a directory, EFAULT for an unreadable
 * path, EACCES for a directory without search permission; a failed call
 * leaves the working directory as it was. */
#include "poly_mtest.h"

int test_main(void) {
    struct poly_stat64 st;
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_mkdir, "locked", 0000);
    check_err("a missing directory: ENOENT", bsd1(SYS_chdir, "missing"), ENOENT);
    check_err("the empty path: ENOENT", bsd1(SYS_chdir, ""), ENOENT);
    check_err("a file: ENOTDIR", bsd1(SYS_chdir, "file"), ENOTDIR);
    check_err("a file as a directory: ENOTDIR", bsd1(SYS_chdir, "file/x"), ENOTDIR);
    check_err("the path at address 8: EFAULT", bsd1(SYS_chdir, (void *)8), EFAULT);
    check_err("a directory with mode 000: EACCES", bsd1(SYS_chdir, "locked"), EACCES);
    check_ok("the working directory is unchanged", bsd2(SYS_stat64, "file", &st));
    bsd2(SYS_chmod, "locked", 0700);
    return finish();
}
