/* open(2), BSD 5: the errors of a path that does not lead to a file that
 * can be opened that way. */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644).ret);
    check_err("a missing file: ENOENT", bsd3(SYS_open, "missing", O_RDONLY, 0), ENOENT);
    check_err("the empty path: ENOENT", bsd3(SYS_open, "", O_RDONLY, 0), ENOENT);
    check_err("O_CREAT in a missing directory: ENOENT", bsd3(SYS_open, "missing/x", O_CREAT | O_WRONLY, 0644), ENOENT);
    check_err("O_DIRECTORY of a file: ENOTDIR", bsd3(SYS_open, "file", O_RDONLY | O_DIRECTORY, 0), ENOTDIR);
    check_err("a file used as a directory: ENOTDIR", bsd3(SYS_open, "file/x", O_RDONLY, 0), ENOTDIR);
    check_err("a directory for writing: EISDIR", bsd3(SYS_open, ".", O_WRONLY, 0), EISDIR);
    check_err("a directory for reading and writing: EISDIR", bsd3(SYS_open, ".", O_RDWR, 0), EISDIR);
    check_err("a path at address 8: EFAULT", bsd3(SYS_open, (void *)8, O_RDONLY, 0), EFAULT);
    return finish();
}
