/* lstat64, BSD 340: describes a symbolic link itself: type S_IFLNK, mode
 * 0755 (a link's permissions are all set), one link, size = the length of
 * the target path; a dangling link and a loop of links can be examined. */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_close, bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0644).ret);
    bsd2(SYS_symlink, "file", "link");
    bsd2(SYS_symlink, "a/much/longer/target", "far");
    bsd2(SYS_symlink, "nowhere", "dangle");
    bsd2(SYS_symlink, "loop2", "loop1");
    bsd2(SYS_symlink, "loop1", "loop2");
    struct poly_stat64 st;
    sysret r = bsd2(SYS_lstat64, "link", &st);
    check("lstat64 of a link returns 0", r.err == 0 && r.ret == 0, r.err);
    check("a symbolic link, mode 0755", st.st_mode == (S_IFLNK | 0755), st.st_mode);
    check("size 4: the length of \"file\"", st.st_size == 4, st.st_size);
    check("one link", st.st_nlink == 1, st.st_nlink);
    bsd2(SYS_lstat64, "far", &st);
    check("size 20: the length of \"a/much/longer/target\"", st.st_size == 20, st.st_size);
    check_ok("a dangling link", bsd2(SYS_lstat64, "dangle", &st));
    check("it is a link of size 7", (st.st_mode & S_IFMT) == S_IFLNK && st.st_size == 7, st.st_size);
    check_ok("a loop of links", bsd2(SYS_lstat64, "loop1", &st));
    return finish();
}
