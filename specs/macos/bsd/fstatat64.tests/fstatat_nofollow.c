/* fstatat64, BSD 470: with flags 0 it follows a final symbolic link, with
 * AT_SYMLINK_NOFOLLOW (0x20) it describes the link (as lstat64). */
#include "poly_mtest.h"

int test_main(void) {
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644);
    bsd3(SYS_write, f.ret, "0123456789", 10);
    bsd2(SYS_symlink, "file", "link");
    bsd2(SYS_mkdir, "dir", 0755);
    sysret d = bsd3(SYS_open, "dir", O_RDONLY, 0);
    bsd2(SYS_symlink, "../file", "dir/up");
    struct poly_stat64 st;
    check_ok("flags 0 on a link", bsd4(SYS_fstatat64, AT_FDCWD, "link", &st, 0));
    check("the file: regular, size 10", (st.st_mode & S_IFMT) == S_IFREG && st.st_size == 10, st.st_size);
    check_ok("AT_SYMLINK_NOFOLLOW on a link", bsd4(SYS_fstatat64, AT_FDCWD, "link", &st, AT_SYMLINK_NOFOLLOW));
    check("the link: S_IFLNK, size 4", (st.st_mode & S_IFMT) == S_IFLNK && st.st_size == 4, st.st_size);
    check_ok("NOFOLLOW from a directory descriptor", bsd4(SYS_fstatat64, d.ret, "up", &st, AT_SYMLINK_NOFOLLOW));
    check("the link: size 7 (\"../file\")", (st.st_mode & S_IFMT) == S_IFLNK && st.st_size == 7, st.st_size);
    check_ok("and following it from the directory descriptor", bsd4(SYS_fstatat64, d.ret, "up", &st, 0));
    check("reaches the file", (st.st_mode & S_IFMT) == S_IFREG && st.st_size == 10, st.st_size);
    return finish();
}
