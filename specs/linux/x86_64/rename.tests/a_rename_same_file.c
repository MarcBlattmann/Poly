/* rename of a name onto itself, or onto another hard link of the same
 * file, succeeds and changes nothing: both names stay, the link count
 * and the timestamps of the file and the directory are unchanged. This
 * holds for files, directories and symbolic links, and even in a
 * directory the caller may not write. A missing name onto itself is
 * ENOENT. */
#include "a_sys.h"

#define D "/tmp/a_rename_same_file"

static i64 rn(const char *a, const char *b) { return syscall2(SYS_rename, a, b); }

static int same_time(const struct a_stat *x, const struct a_stat *y) {
    return x->st_mtime_sec == y->st_mtime_sec && x->st_mtime_nsec == y->st_mtime_nsec &&
           x->st_ctime_sec == y->st_ctime_sec && x->st_ctime_nsec == y->st_ctime_nsec;
}

static void nap(void) {
    struct { i64 s, ns; } ts = {0, 30000000};
    syscall2(SYS_nanosleep, &ts, 0);
}

int test_main(void) {
    struct a_stat f0, d0, f1, d1, st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_f", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_link", syscall5(SYS_linkat, AT_FDCWD, D "/f", AT_FDCWD, D "/h", 0), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_sym", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/s"), 0);
    a_stat_path(D "/f", &f0);
    a_stat_path(D, &d0);
    nap();

    expect("file_onto_itself", rn(D "/f", D "/f"), 0);
    expect("file_onto_hard_link", rn(D "/f", D "/h"), 0);
    expect("hard_link_onto_file", rn(D "/h", D "/f"), 0);
    expect("f_still_there", a_stat_path(D "/f", &f1), 0);
    expect("h_still_there", a_stat_path(D "/h", &st), 0);
    check("same_inode", st.st_ino == f1.st_ino, 0);
    expect("nlink_still_2", f1.st_nlink, 2);
    check("file_times_unchanged", same_time(&f0, &f1), 0);
    a_stat_path(D, &d1);
    check("dir_times_unchanged", same_time(&d0, &d1), 0);

    expect("dir_onto_itself", rn(D "/sub", D "/sub"), 0);
    expect("dir_onto_itself_slashes", rn(D "/sub/", D "/sub/"), 0);
    expect("sub_still_there", a_stat_path(D "/sub", &st), 0);
    expect("symlink_onto_itself", rn(D "/s", D "/s"), 0);
    expect("symlink_still_link", a_lstat_path(D "/s", &st) == 0 && (st.st_mode & S_IFMT) == S_IFLNK, 1);
    a_stat_path(D, &d1);
    check("dir_times_still_unchanged", same_time(&d0, &d1), 0);

    expect("missing_onto_itself", rn(D "/none", D "/none"), -ENOENT);

    expect("chmod_dir_555", syscall3(SYS_fchmodat, AT_FDCWD, D, 0555), 0);
    expect("onto_itself_in_read_only_dir", rn(D "/f", D "/f"), 0);
    expect("onto_link_in_read_only_dir", rn(D "/h", D "/f"), 0);
    expect("chmod_dir_755", syscall3(SYS_fchmodat, AT_FDCWD, D, 0755), 0);

    expect("cleanup_s", a_unlink(D "/s"), 0);
    expect("cleanup_h", a_unlink(D "/h"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
