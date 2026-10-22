/* rmdir only removes directories. The last name is never followed if it
 * is a symbolic link: a link to a directory, a link to a file and a
 * dangling link are all ENOTDIR, with or without a trailing slash, and
 * the link and its target are left alone. A regular file or FIFO is
 * ENOTDIR; with a trailing slash too. Symbolic links before the last
 * name are followed: rmdir("link/sub") removes sub inside the target. */
#include "a_sys.h"

#define D "/tmp/c_rmdir_notdir"
#define SYS_mknodat 259

static i64 rm(const char *p) { return syscall1(SYS_rmdir, p); }
static i64 sl(const char *target, const char *path) { return syscall3(SYS_symlinkat, target, AT_FDCWD, path); }

int test_main(void) {
    struct a_stat st;
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/dir", 0755), 0);
    expect("setup_file", a_make_file(D "/file", "x", 0644), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, D "/fifo", S_IFIFO | 0644, 0), 0);
    expect("setup_ldir", sl("dir", D "/ldir"), 0);
    expect("setup_ldir_abs", sl(D "/dir", D "/ldir_abs"), 0);
    expect("setup_lfile", sl("file", D "/lfile"), 0);
    expect("setup_ldang", sl("nowhere", D "/ldang"), 0);
    expect("setup_lloop", sl("lloop", D "/lloop"), 0);

    expect("file_enotdir", rm(D "/file"), -ENOTDIR);
    expect("file_slash_enotdir", rm(D "/file/"), -ENOTDIR);
    expect("file_kept", a_lstat_path(D "/file", &st) == 0 && (st.st_mode & S_IFMT) == S_IFREG, 1);
    expect("fifo_enotdir", rm(D "/fifo"), -ENOTDIR);
    expect("link_to_dir_enotdir", rm(D "/ldir"), -ENOTDIR);
    expect("abs_link_to_dir_enotdir", rm(D "/ldir_abs"), -ENOTDIR);
    expect("link_to_dir_slash_enotdir", rm(D "/ldir/"), -ENOTDIR);
    expect("link_to_dir_slashes_enotdir", rm(D "/ldir//"), -ENOTDIR);
    expect("link_kept", a_lstat_path(D "/ldir", &st) == 0 && (st.st_mode & S_IFMT) == S_IFLNK, 1);
    expect("link_target_kept", a_lstat_path(D "/dir", &st) == 0 && (st.st_mode & S_IFMT) == S_IFDIR, 1);
    expect("link_to_file_enotdir", rm(D "/lfile"), -ENOTDIR);
    expect("link_to_file_slash_enotdir", rm(D "/lfile/"), -ENOTDIR);
    expect("dangling_link_enotdir", rm(D "/ldang"), -ENOTDIR);
    expect("dangling_link_slash_enotdir", rm(D "/ldang/"), -ENOTDIR);
    expect("dangling_target_not_made", a_lstat_path(D "/nowhere", &st), -ENOENT);
    expect("loop_link_last_enotdir", rm(D "/lloop"), -ENOTDIR);

    /* Links on the way are followed. */
    expect("mkdir_sub", a_mkdir(D "/dir/sub", 0755), 0);
    expect("rmdir_through_link", rm(D "/ldir/sub"), 0);
    expect("sub_gone", a_lstat_path(D "/dir/sub", &st), -ENOENT);
    expect("mkdir_sub2", a_mkdir(D "/dir/sub2", 0755), 0);
    expect("rmdir_through_abs_link", rm(D "/ldir_abs/sub2"), 0);
    expect("mkdir_sub3", a_mkdir(D "/dir/sub3", 0755), 0);
    expect("rmdir_through_link_dot", rm(D "/ldir/./sub3/"), 0);

    /* A non-directory on the way. */
    expect("file_on_way_enotdir", rm(D "/file/x"), -ENOTDIR);
    expect("lfile_on_way_enotdir", rm(D "/lfile/x"), -ENOTDIR);
    expect("dangling_on_way_enoent", rm(D "/ldang/x"), -ENOENT);
    expect("loop_on_way_eloop", rm(D "/lloop/x"), -ELOOP);

    expect("cleanup_links", a_unlink(D "/ldir") | a_unlink(D "/ldir_abs") | a_unlink(D "/lfile") |
                                a_unlink(D "/ldang") | a_unlink(D "/lloop"), 0);
    expect("cleanup_files", a_unlink(D "/file") | a_unlink(D "/fifo"), 0);
    expect("cleanup_dir", rm(D "/dir"), 0);
    expect("cleanup_rmdir", rm(D), 0);
    return finish();
}
