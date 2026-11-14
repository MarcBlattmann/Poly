/* stat64, BSD 338 (path, struct stat64): the fields of a file named by
 * path, as fstat64 gives them for its descriptor: type and mode (0644 less
 * umask 022), one link, size, owner. A directory on APFS has one link per
 * entry plus 2 ("." and ".."), and 32 bytes per entry including those two. */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    sysret f = bsd3(SYS_open, "file", O_CREAT | O_WRONLY, 0666);
    bsd3(SYS_write, f.ret, "0123456789", 10);
    bsd2(SYS_mkdir, "dir", 0755);
    struct poly_stat64 st, fs;
    memset(&st, 0xa5, sizeof st);
    sysret r = bsd2(SYS_stat64, "file", &st);
    check("stat64 returns 0", r.err == 0 && r.ret == 0, r.err);
    check("a regular file, mode 0644", st.st_mode == (S_IFREG | 0644), st.st_mode);
    check("one link, size 10", st.st_nlink == 1 && st.st_size == 10, st.st_size);
    check("the caller owns it", st.st_uid == (u32)bsd0(SYS_getuid).ret && st.st_gid == (u32)bsd0(SYS_getgid).ret, st.st_uid);
    bsd2(SYS_fstat64, f.ret, &fs);
    check("the same inode and size as fstat64 of the open file", st.st_ino == fs.st_ino && st.st_size == fs.st_size, 0);
    check_ok("stat64 of \".\"", bsd2(SYS_stat64, ".", &st));
    check("the working directory: a directory, 4 links (2 entries + 2)", (st.st_mode & S_IFMT) == S_IFDIR && st.st_nlink == 4, st.st_nlink);
    check("and size 128 (4 x 32)", st.st_size == 128, st.st_size);
    check_ok("stat64 of \"dir/\" with a trailing slash", bsd2(SYS_stat64, "dir/", &st));
    check("an empty directory: mode 0755, 2 links, size 64", st.st_mode == (S_IFDIR | 0755) && st.st_nlink == 2 && st.st_size == 64, st.st_size);
    return finish();
}
