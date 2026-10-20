/* statx: the basic fields of a file, directory and symbolic link agree
 * with newfstatat; the returned mask covers the basic statistics. */
#include "a_sys.h"

#define D "/tmp/a_statx_basic"

static i64 statx(i64 dfd, const char *p, i64 flags, i64 mask, struct a_statx *sx) {
    return syscall5(SYS_statx, dfd, p, flags, mask, sx);
}

int test_main(void) {
    struct a_statx sx;
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0710), 0);
    expect("setup_file", a_make_file(D "/f", "123456789", 0640), 0);
    expect("setup_symlink", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/l"), 0);

    memset(&sx, 0xff, sizeof sx);
    expect("statx_file", statx(AT_FDCWD, D "/f", 0, STATX_BASIC_STATS, &sx), 0);
    a_stat_path(D "/f", &st);
    expect("mask_has_basic_stats", sx.stx_mask & STATX_BASIC_STATS, STATX_BASIC_STATS);
    expect("mode", sx.stx_mode, S_IFREG | 0640);
    expect("size", sx.stx_size, 9);
    expect("nlink", sx.stx_nlink, 1);
    expect("uid", sx.stx_uid, st.st_uid);
    expect("gid", sx.stx_gid, st.st_gid);
    check("ino_matches_stat", sx.stx_ino == st.st_ino, 0);
    expect("blocks_match_stat", sx.stx_blocks, st.st_blocks);
    expect("blksize_matches_stat", sx.stx_blksize, st.st_blksize);
    check("mtime_matches_stat", sx.stx_mtime.tv_sec == (i64)st.st_mtime_sec && sx.stx_mtime.tv_nsec == st.st_mtime_nsec, 0);
    check("ctime_matches_stat", sx.stx_ctime.tv_sec == (i64)st.st_ctime_sec && sx.stx_ctime.tv_nsec == st.st_ctime_nsec, 0);
    expect("rdev_major_0", sx.stx_rdev_major, 0);
    expect("rdev_minor_0", sx.stx_rdev_minor, 0);
    expect("spare0_zeroed", sx.spare0, 0);

    expect("statx_dir", statx(AT_FDCWD, D, 0, STATX_BASIC_STATS, &sx), 0);
    expect("dir_mode", sx.stx_mode, S_IFDIR | 0710);
    expect("statx_symlink_followed", statx(AT_FDCWD, D "/l", 0, STATX_BASIC_STATS, &sx), 0);
    expect("followed_mode", sx.stx_mode, S_IFREG | 0640);
    expect("statx_symlink_nofollow", statx(AT_FDCWD, D "/l", AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, &sx), 0);
    expect("symlink_mode", sx.stx_mode, S_IFLNK | 0777);
    expect("symlink_size", sx.stx_size, 1);

    i64 fd = a_open("/dev/null", O_RDONLY, 0);
    expect("statx_devnull_empty_path", statx(fd, "", AT_EMPTY_PATH, STATX_BASIC_STATS, &sx), 0);
    expect("devnull_char_device", sx.stx_mode & S_IFMT, S_IFCHR);
    expect("devnull_rdev_major_1", sx.stx_rdev_major, 1);
    expect("devnull_rdev_minor_3", sx.stx_rdev_minor, 3);
    a_close(fd);

    expect("cleanup_l", a_unlink(D "/l"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
