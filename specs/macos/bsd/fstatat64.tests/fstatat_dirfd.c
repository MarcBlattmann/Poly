/* fstatat64, BSD 470 (dirfd, path, struct stat64, flags): a relative path
 * is resolved from the directory dirfd, or from the working directory when
 * dirfd is AT_FDCWD (-2 on Darwin); an absolute path ignores dirfd (even a
 * descriptor that is not open). */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    bsd1(SYS_close, bsd3(SYS_open, "top", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_mkdir, "dir", 0755);
    sysret w = bsd3(SYS_open, "dir/inner", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, w.ret, "xy", 2);
    sysret d = bsd3(SYS_open, "dir", O_RDONLY, 0);
    struct poly_stat64 st;
    sysret r = bsd4(SYS_fstatat64, d.ret, "inner", &st, 0);
    check("a relative path from a directory descriptor", r.err == 0 && r.ret == 0 && st.st_size == 2, r.err);
    check_err("top is not in dir: ENOENT", bsd4(SYS_fstatat64, d.ret, "top", &st, 0), ENOENT);
    check_ok("AT_FDCWD (-2): top is in the working directory", bsd4(SYS_fstatat64, AT_FDCWD, "top", &st, 0));
    check("a regular file of size 0", (st.st_mode & S_IFMT) == S_IFREG && st.st_size == 0, st.st_size);
    check_ok("an absolute path ignores dirfd", bsd4(SYS_fstatat64, d.ret, "/dev/null", &st, 0));
    check("it is /dev/null", st.st_mode == (S_IFCHR | 0666), st.st_mode);
    check_ok("an absolute path with a descriptor that is not open", bsd4(SYS_fstatat64, 99, "/dev/null", &st, 0));
    check_ok("a path in the middle: dir/inner from AT_FDCWD", bsd4(SYS_fstatat64, AT_FDCWD, "dir/inner", &st, 0));
    return finish();
}
