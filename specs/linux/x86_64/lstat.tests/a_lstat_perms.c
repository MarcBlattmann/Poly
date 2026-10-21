/* lstat: needs no permission on the file itself, only search (x)
 * permission on every directory of the path. A directory without x
 * refuses with EACCES unless the caller is root (who passes the check). */
#include "a_sys.h"

#define D "/tmp/a_lstat_perms"

static i64 lstat(const char *p, struct a_stat *st) { return syscall2(SYS_lstat, p, st); }
static i64 chmod(const char *p, i64 m) { return syscall3(SYS_fchmodat, AT_FDCWD, p, m); }

int test_main(void) {
    struct a_stat st;
    int root = syscall0(SYS_geteuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "secret", 0000), 0);
    expect("mode_000_file_ok", lstat(D "/f", &st), 0);
    expect("mode_000_file_mode", st.st_mode, S_IFREG | 0);
    expect("mode_000_file_size", st.st_size, 6);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner_file", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_inner_link", syscall3(SYS_symlinkat, "g", AT_FDCWD, D "/sub/l"), 0);

    expect("chmod_sub_000", chmod(D "/sub", 0), 0);
    expect("dir_itself_ok", lstat(D "/sub", &st), 0);
    expect("dir_itself_mode", st.st_mode, S_IFDIR | 0);
    expect("child_no_search", lstat(D "/sub/g", &st), root ? 0 : -EACCES);
    expect("link_no_search", lstat(D "/sub/l", &st), root ? 0 : -EACCES);
    expect("missing_no_search", lstat(D "/sub/nope", &st), root ? -ENOENT : -EACCES);
    expect("dir_slash_ok", lstat(D "/sub/", &st), 0);

    expect("chmod_sub_600", chmod(D "/sub", 0600), 0);
    expect("child_read_no_search", lstat(D "/sub/g", &st), root ? 0 : -EACCES);

    expect("chmod_sub_100", chmod(D "/sub", 0100), 0);
    expect("child_search_only_ok", lstat(D "/sub/g", &st), 0);
    expect("link_search_only_ok", lstat(D "/sub/l", &st), 0);
    expect("link_search_only_type", st.st_mode & S_IFMT, S_IFLNK);

    expect("chmod_sub_755", chmod(D "/sub", 0755), 0);
    expect("cleanup_l", a_unlink(D "/sub/l"), 0);
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
