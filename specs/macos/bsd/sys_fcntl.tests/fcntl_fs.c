#include "poly_mtest.h"
/* fcntl, BSD 92: file-system commands. F_FULLFSYNC (51) returns 0 for a regular file and a directory,
 * ENODEV (19) for /dev/null and EBADF (9) for a pipe; F_BARRIERFSYNC (85), F_NOCACHE (48, argument 1
 * or 0), F_GLOBAL_NOCACHE (55) and F_RDAHEAD (45) return 0 on a file; F_NOCACHE and F_RDAHEAD on a
 * pipe are EBADF. F_GETPATH (50) stores the absolute path (buffer of 1024 bytes, MAXPATHLEN) of an open
 * regular file or directory, "/dev/null" for the null device, EBADF for a pipe or a descriptor that is not
 * open, EFAULT for an unwritable or NULL buffer; the path follows a rename of the file and an unlinked
 * file still succeeds; F_GETPATH_NOFIRMLINK (102) works too; F_PREALLOCATE (42) with zeros is
 * EINVAL (22). */
#define F_NOCACHE 48
#define F_LOG2PHYS 49
#define F_FULLFSYNC 51
#define F_GLOBAL_NOCACHE 55
#define F_RDAHEAD 45
#define F_BARRIERFSYNC 85
#define F_GETPATH_NOFIRMLINK 102
#define F_PREALLOCATE 42
#define SYS_rename 128
#define SYS_unlink 10
int test_main(void) {
    int fd = bsd3(SYS_open, "file", O_CREAT | O_RDWR, 0644).ret;
    bsd3(SYS_write, fd, "data", 4);
    check_ok("F_FULLFSYNC on a regular file", bsd3(SYS_fcntl, fd, F_FULLFSYNC, 0));
    check("it returns 0", bsd3(SYS_fcntl, fd, F_FULLFSYNC, 0).ret == 0, 0);
    int dir = bsd3(SYS_open, ".", O_RDONLY, 0).ret;
    check_ok("F_FULLFSYNC on a directory", bsd3(SYS_fcntl, dir, F_FULLFSYNC, 0));
    int nul = bsd3(SYS_open, "/dev/null", O_RDWR, 0).ret;
    check_err("F_FULLFSYNC on /dev/null is ENODEV", bsd3(SYS_fcntl, nul, F_FULLFSYNC, 0), ENODEV);
    sysret p = bsd0(SYS_pipe);
    check_err("F_FULLFSYNC on a pipe is EBADF", bsd3(SYS_fcntl, p.ret, F_FULLFSYNC, 0), EBADF);
    check_ok("F_BARRIERFSYNC on a file", bsd3(SYS_fcntl, fd, F_BARRIERFSYNC, 0));
    check_ok("F_NOCACHE on", bsd3(SYS_fcntl, fd, F_NOCACHE, 1));
    check_ok("F_NOCACHE off", bsd3(SYS_fcntl, fd, F_NOCACHE, 0));
    check_ok("F_GLOBAL_NOCACHE", bsd3(SYS_fcntl, fd, F_GLOBAL_NOCACHE, 1));
    check_ok("F_RDAHEAD on", bsd3(SYS_fcntl, fd, F_RDAHEAD, 1));
    check_err("F_NOCACHE on a pipe is EBADF", bsd3(SYS_fcntl, p.ret, F_NOCACHE, 1), EBADF);
    check_err("F_RDAHEAD on a pipe is EBADF", bsd3(SYS_fcntl, p.ret, F_RDAHEAD, 1), EBADF);
    char path[1100];
    memset(path, 0, sizeof path);
    sysret r = bsd3(SYS_fcntl, fd, F_GETPATH, path);
    size_t n = str_len(path);
    check("F_GETPATH: 0 and an absolute path ending in /file", r.err == 0 && r.ret == 0 && path[0] == '/' && n >= 5 && str_eq(path + n - 5, "/file"), r.err);
    char path2[1100];
    memset(path2, 0, sizeof path2);
    r = bsd3(SYS_fcntl, fd, F_GETPATH_NOFIRMLINK, path2);
    check("F_GETPATH_NOFIRMLINK: an absolute path ending in /file too", r.err == 0 && path2[0] == '/' && str_len(path2) >= 5 && str_eq(path2 + str_len(path2) - 5, "/file"), r.err);
    memset(path, 0, sizeof path);
    check_ok("F_GETPATH of a directory", bsd3(SYS_fcntl, dir, F_GETPATH, path));
    check("it is absolute and does not end in a slash", path[0] == '/' && path[str_len(path) - 1] != '/', 0);
    memset(path, 0, sizeof path);
    check_ok("F_GETPATH of /dev/null", bsd3(SYS_fcntl, nul, F_GETPATH, path));
    check("the path is /dev/null", str_eq(path, "/dev/null"), 0);
    check_err("F_GETPATH of a pipe is EBADF", bsd3(SYS_fcntl, p.ret, F_GETPATH, path), EBADF);
    check_err("F_GETPATH of a descriptor not open is EBADF", bsd3(SYS_fcntl, 99, F_GETPATH, path), EBADF);
    check_err("F_GETPATH with an unwritable buffer is EFAULT", bsd3(SYS_fcntl, fd, F_GETPATH, 8), EFAULT);
    check_err("F_GETPATH with a NULL buffer is EFAULT", bsd3(SYS_fcntl, fd, F_GETPATH, 0), EFAULT);
    bsd2(SYS_rename, "file", "renamed");
    memset(path, 0, sizeof path);
    bsd3(SYS_fcntl, fd, F_GETPATH, path);
    n = str_len(path);
    check("after a rename F_GETPATH shows the new name", n >= 8 && str_eq(path + n - 8, "/renamed"), n);
    bsd1(SYS_unlink, "renamed");
    memset(path, 0, sizeof path);
    r = bsd3(SYS_fcntl, fd, F_GETPATH, path);
    check("F_GETPATH of an unlinked file still succeeds", r.err == 0, r.err);
    char zeros[64];
    memset(zeros, 0, sizeof zeros);
    value("F_LOG2PHYS with zeros on a file with data: errno", bsd3(SYS_fcntl, fd, F_LOG2PHYS, zeros).err);
    check_err("F_PREALLOCATE with zeros is EINVAL", bsd3(SYS_fcntl, fd, F_PREALLOCATE, zeros), EINVAL);
    return finish();
}
