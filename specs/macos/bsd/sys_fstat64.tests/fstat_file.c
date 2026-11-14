/* fstat64(2), BSD 339: a regular file's type and mode, link count, size,
 * owner and inode; struct stat64 of <sys/stat.h>. The block size and
 * block count are those of the oracle's APFS volume (values). */
#include "poly_mtest.h"

int test_main(void) {
    bsd1(SYS_umask, 022);
    sysret f = bsd3(SYS_open, "s.txt", O_CREAT | O_RDWR, 0644);
    bsd3(SYS_write, f.ret, "hello", 5);
    struct poly_stat64 st;
    memset(&st, 0xa5, sizeof st);
    sysret r = bsd2(SYS_fstat64, f.ret, &st);
    check("fstat64 returns 0", r.err == 0 && r.ret == 0, r.err);
    check("a regular file, mode 0644", st.st_mode == (S_IFREG | 0644), st.st_mode);
    check("one link", st.st_nlink == 1, st.st_nlink);
    check("size 5", st.st_size == 5, st.st_size);
    check("an inode number", st.st_ino != 0, 0);
    check("owned by the caller's uid", st.st_uid == (u32)bsd0(SYS_getuid).ret, st.st_uid);
    check("and gid", st.st_gid == (u32)bsd0(SYS_getgid).ret, st.st_gid);
    check("no flags", st.st_flags == 0, st.st_flags);
    check("st_gen is 0 for an unprivileged caller", st.st_gen == 0, st.st_gen);
    check("modified after 2023", st.st_mtimespec.tv_sec > 1700000000, 0);
    value("st_blksize", st.st_blksize);
    value("st_blocks", st.st_blocks);
    return finish();
}
