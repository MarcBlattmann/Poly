/* rename(2) of non-directories: the name moves, the inode stays; an
 * existing target is replaced atomically (the replaced file lives on in
 * open descriptors); renaming onto itself or onto another name of the
 * same file succeeds and changes nothing; symbolic links are moved, not
 * followed; the moved inode's ctime and both parents' mtime change. */
#include "a_sys.h"

#define D "/tmp/lb_rename_files"

static i64 rename_(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }
static i64 ino_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_ino;
}
static i64 nlink_of(const char *p) {
    struct a_stat st;
    if (a_lstat_path(p, &st)) return -1;
    return (i64)st.st_nlink;
}

int test_main(void) {
    struct a_stat st, st2;
    char c[16];
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_a", a_mkdir(D "/a", 0755), 0);
    expect("setup_b", a_mkdir(D "/b", 0755), 0);
    expect("setup_f", a_make_file(D "/a/f", "first", 0644), 0);
    i64 ino_f = ino_of(D "/a/f");

    /* simple move within a directory and across directories */
    expect("rename_same_dir", rename_(D "/a/f", D "/a/g"), 0);
    expect("old_gone", a_lstat_path(D "/a/f", &st), -ENOENT);
    expect("new_same_inode", ino_of(D "/a/g"), ino_f);
    expect("rename_across", rename_(D "/a/g", D "/b/f"), 0);
    expect("across_same_inode", ino_of(D "/b/f"), ino_f);
    expect("nlink_still_1", nlink_of(D "/b/f"), 1);

    /* replacing an existing file */
    expect("setup_victim", a_make_file(D "/b/victim", "victim", 0600), 0);
    i64 vfd = a_open(D "/b/victim", O_RDONLY, 0);
    check("open_victim", vfd >= 0, vfd);
    expect("replace", rename_(D "/b/f", D "/b/victim"), 0);
    expect("replaced_inode", ino_of(D "/b/victim"), ino_f);
    expect("replaced_mode", a_stat_path(D "/b/victim", &st) == 0 && st.st_mode == (S_IFREG | 0644), 1);
    expect("victim_fd_nlink_0", syscall2(SYS_fstat, vfd, &st) == 0 && st.st_nlink == 0, 1);
    expect("victim_fd_reads", syscall4(SYS_pread64, vfd, c, 16, 0), 6);
    expect("victim_contents", mem_eq(c, "victim", 6), 1);
    expect("close_victim", a_close(vfd), 0);

    /* onto itself, and onto another hard link of the same file: no change */
    expect("onto_itself", rename_(D "/b/victim", D "/b/victim"), 0);
    expect("link_h", syscall2(SYS_link, D "/b/victim", D "/b/h"), 0);
    expect("onto_hard_link", rename_(D "/b/victim", D "/b/h"), 0);
    expect("both_names_remain", ino_of(D "/b/victim") == ino_f && ino_of(D "/b/h") == ino_f, 1);
    expect("nlink_2", nlink_of(D "/b/h"), 2);
    expect("unlink_h", a_unlink(D "/b/h"), 0);

    /* symbolic links move themselves; the target is untouched */
    expect("setup_link", syscall2(SYS_symlink, "victim", D "/b/l"), 0);
    i64 ino_l = ino_of(D "/b/l");
    expect("rename_link", rename_(D "/b/l", D "/a/l"), 0);
    expect("link_moved", ino_of(D "/a/l"), ino_l);
    expect("link_now_dangling", a_stat_path(D "/a/l", &st), -ENOENT);
    expect("target_kept", ino_of(D "/b/victim"), ino_f);
    expect("setup_dangling", syscall2(SYS_symlink, "nowhere", D "/a/dangling"), 0);
    expect("rename_dangling", rename_(D "/a/dangling", D "/a/d2"), 0);
    /* a link replaced by a file, a file replaced by a link */
    expect("file_over_link", rename_(D "/b/victim", D "/a/l"), 0);
    expect("file_over_link_inode", ino_of(D "/a/l"), ino_f);
    expect("link_over_file", rename_(D "/a/d2", D "/a/l"), 0);
    expect("link_over_file_kind", a_lstat_path(D "/a/l", &st) == 0 && (st.st_mode & S_IFMT) == S_IFLNK, 1);
    /* renaming the link to a directory moves the link, not the directory */
    expect("setup_dirlink", syscall2(SYS_symlink, "b", D "/dl"), 0);
    expect("rename_dirlink", rename_(D "/dl", D "/dl2"), 0);
    expect("b_still_there", a_lstat_path(D "/b", &st) == 0 && (st.st_mode & S_IFMT) == S_IFDIR, 1);
    expect("unlink_dl2", a_unlink(D "/dl2"), 0);

    /* times: the moved inode's ctime, the parents' mtime/ctime */
    expect("setup_t", a_make_file(D "/a/t", "t", 0644), 0);
    struct a_timespec ts[2] = {{1000000000, 0}, {1000000000, 0}};
    expect("set_t", syscall4(SYS_utimensat, AT_FDCWD, D "/a/t", ts, 0), 0);
    expect("set_a", syscall4(SYS_utimensat, AT_FDCWD, D "/a", ts, 0), 0);
    expect("set_b", syscall4(SYS_utimensat, AT_FDCWD, D "/b", ts, 0), 0);
    expect("stat_t0", a_stat_path(D "/a/t", &st), 0);
    struct a_timespec pause = {0, 20000000};
    syscall2(SYS_nanosleep, &pause, 0);
    expect("rename_t", rename_(D "/a/t", D "/b/t"), 0);
    expect("stat_t1", a_stat_path(D "/b/t", &st2), 0);
    check("moved_ctime_changed",
          st2.st_ctime_sec > st.st_ctime_sec || (st2.st_ctime_sec == st.st_ctime_sec && st2.st_ctime_nsec > st.st_ctime_nsec), 0);
    expect("moved_mtime_kept", st2.st_mtime_sec, 1000000000);
    expect("moved_atime_kept", st2.st_atime_sec, 1000000000);
    expect("stat_a", a_stat_path(D "/a", &st), 0);
    check("old_parent_mtime_changed", st.st_mtime_sec > 1000000000, 0);
    expect("stat_b", a_stat_path(D "/b", &st), 0);
    check("new_parent_mtime_changed", st.st_mtime_sec > 1000000000, 0);

    /* relative paths */
    expect("chdir", syscall1(SYS_chdir, D "/a"), 0);
    expect("relative", rename_("../b/t", "t2"), 0);
    expect("relative_result", a_lstat_path(D "/a/t2", &st), 0);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    expect("cleanup_t2", a_unlink(D "/a/t2"), 0);
    expect("cleanup_l", a_unlink(D "/a/l"), 0);
    expect("cleanup_a", a_rmdir(D "/a"), 0);
    expect("cleanup_b", a_rmdir(D "/b"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
