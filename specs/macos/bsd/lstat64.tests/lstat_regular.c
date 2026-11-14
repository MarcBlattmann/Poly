/* lstat64, BSD 340: for everything that is not a symbolic link it gives
 * what stat64 gives: regular file, directory, device. Only the last
 * component is not followed: a link in the middle of the path is. */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, f.ret, "abc", 3);
    bsd2(SYS_mkdir, "dir", 0755);
    bsd1(SYS_close, bsd3(SYS_open, "dir/in", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_symlink, "dir", "dlink");
    struct poly_stat64 a, b;
    bsd2(SYS_lstat64, "file", &a);
    bsd2(SYS_stat64, "file", &b);
    check("a file: the same as stat64", mem_eq(&a.st_ino, &b.st_ino, 8) && a.st_mode == b.st_mode && a.st_size == b.st_size, a.st_mode);
    check("a regular file, size 3", a.st_mode == (S_IFREG | 0644) && a.st_size == 3, a.st_size);
    bsd2(SYS_lstat64, "dir", &a);
    check("a directory", a.st_mode == (S_IFDIR | 0755), a.st_mode);
    bsd2(SYS_lstat64, "/dev/null", &a);
    check("a device", a.st_mode == (S_IFCHR | 0666), a.st_mode);
    check_ok("a link in the middle of the path is followed", bsd2(SYS_lstat64, "dlink/in", &a));
    check("and it is the file inside", a.st_mode == (S_IFREG | 0644), a.st_mode);
    bsd2(SYS_lstat64, "dlink/", &a);
    check("a trailing slash follows the link too: a directory", (a.st_mode & S_IFMT) == S_IFDIR, a.st_mode);
    return finish();
}
