/* fadvise64 changes nothing stat can see: size, blocks, mode, link count
 * and all three timestamps (atime too, even for WILLNEED) stay the same
 * after every advice; so do the directory's. */
#include "a_sys.h"

#define SYS_fadvise64 221
#define D "/tmp/fadvise64_stat_unchanged"

static i64 fadv(i64 fd, i64 off, i64 len, i64 advice) { return syscall4(SYS_fadvise64, fd, off, len, advice); }

static int same(const struct a_stat *a, const struct a_stat *b) {
    return a->st_ino == b->st_ino && a->st_size == b->st_size && a->st_blocks == b->st_blocks &&
           a->st_mode == b->st_mode && a->st_nlink == b->st_nlink && a->st_atime_sec == b->st_atime_sec &&
           a->st_atime_nsec == b->st_atime_nsec && a->st_mtime_sec == b->st_mtime_sec &&
           a->st_mtime_nsec == b->st_mtime_nsec && a->st_ctime_sec == b->st_ctime_sec &&
           a->st_ctime_nsec == b->st_ctime_nsec;
}

int test_main(void) {
    static char buf[5 * 4096];
    memset(buf, 's', sizeof buf);
    a_mkdir(D, 0755);
    i64 w = a_open(D "/f", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    check("create", w >= 0, w);
    expect("write", a_write(w, buf, sizeof buf), sizeof buf);
    a_close(w);
    struct a_stat s0, s1, d0, d1;
    expect("stat_before", a_stat_path(D "/f", &s0), 0);
    expect("dir_stat_before", a_stat_path(D, &d0), 0);
    i64 fd = a_open(D "/f", O_RDONLY, 0);
    check("open", fd >= 0, fd);
    int ok = 1;
    for (i64 a = 0; a <= 5; a++) ok &= fadv(fd, 0, 0, a) == 0 && fadv(fd, 4096, 8192, a) == 0;
    check("all_advice", ok, 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open_dir", dfd >= 0, dfd);
    ok = 1;
    for (i64 a = 0; a <= 5; a++) ok &= fadv(dfd, 0, 0, a) == 0;
    check("dir_all_advice", ok, 0);
    expect("stat_after", a_stat_path(D "/f", &s1), 0);
    expect("dir_stat_after", a_stat_path(D, &d1), 0);
    check("file_unchanged", same(&s0, &s1), s1.st_size);
    check("dir_unchanged", same(&d0, &d1), d1.st_size);
    a_close(fd);
    a_close(dfd);
    a_unlink(D "/f");
    a_rmdir(D);
    return finish();
}
