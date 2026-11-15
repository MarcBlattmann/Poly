/* stat64, BSD 338, errors: ENOENT for a missing file and the empty path,
 * ENOTDIR for a file used as a directory and for a file with a trailing
 * slash, ENAMETOOLONG for a component over 255 bytes, EFAULT for the path
 * and for the result buffer; special files: /dev/null and "/". */
#include "poly_mtest.h"

static char longname[300];

int test_main(void) {
    struct poly_stat64 st;
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644).ret);
    check_err("a missing file: ENOENT", bsd2(SYS_stat64, "missing", &st), ENOENT);
    check_err("the empty path: ENOENT", bsd2(SYS_stat64, "", &st), ENOENT);
    check_err("a file as a directory: ENOTDIR", bsd2(SYS_stat64, "file/x", &st), ENOTDIR);
    check_err("a file with a trailing slash: ENOTDIR", bsd2(SYS_stat64, "file/", &st), ENOTDIR);
    for (int i = 0; i < 299; i++) longname[i] = 'n';
    check_err("a 299-byte name: ENAMETOOLONG", bsd2(SYS_stat64, longname, &st), ENAMETOOLONG);
    check_err("the path at address 8: EFAULT", bsd2(SYS_stat64, (void *)8, &st), EFAULT);
    check_err("the buffer at address 8: EFAULT", bsd2(SYS_stat64, "file", (void *)8), EFAULT);
    check_ok("stat64 of /dev/null", bsd2(SYS_stat64, "/dev/null", &st));
    check("a character device 3,2, mode 0666", st.st_mode == (S_IFCHR | 0666) && st.st_rdev == ((3 << 24) | 2), st.st_mode);
    check_ok("stat64 of /", bsd2(SYS_stat64, "/", &st));
    check("a directory, owned by root", (st.st_mode & S_IFMT) == S_IFDIR && st.st_uid == 0, st.st_uid);
    return finish();
}
