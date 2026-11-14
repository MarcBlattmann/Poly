/* fcntl, BSD 92: F_GETPATH (50) writes the path of the open file into a
 * buffer of 1024 bytes ("/dev/null" for /dev/null, a path ending in the
 * directory's name for a directory); EBADF for a descriptor that is not
 * open, EFAULT for an unwritable buffer. Values: the errno of a command
 * that does not exist. */
#include "poly_mtest.h"

int test_main(void) {
    char path[1024];
    memset(path, 0, sizeof path);
    sysret n = bsd3(SYS_open, "/dev/null", O_RDONLY, 0);
    check_ok("F_GETPATH of /dev/null", bsd3(SYS_fcntl, n.ret, F_GETPATH, path));
    check("it is \"/dev/null\"", str_eq(path, "/dev/null"), path[0]);
    bsd2(SYS_mkdir, "somedir", 0755);
    sysret d = bsd3(SYS_open, "somedir", O_RDONLY, 0);
    memset(path, 0, sizeof path);
    check_ok("F_GETPATH of a directory", bsd3(SYS_fcntl, d.ret, F_GETPATH, path));
    size_t len = str_len(path);
    check("an absolute path ending in /somedir", path[0] == '/' && len > 8 && str_eq(path + len - 8, "/somedir"), len);
    check_err("F_GETPATH of a closed descriptor: EBADF", bsd3(SYS_fcntl, 77, F_GETPATH, path), EBADF);
    check_err("F_GETPATH into address 8: EFAULT", bsd3(SYS_fcntl, d.ret, F_GETPATH, (void *)8), EFAULT);
    check_err("F_GETFD of a closed descriptor: EBADF", bsd3(SYS_fcntl, 77, F_GETFD, 0), EBADF);
    sysret bad = bsd3(SYS_fcntl, n.ret, 999, 0);
    value("command 999 on an open file: errno", bad.err);
    return finish();
}
