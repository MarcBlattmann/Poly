/* mkdir fails with EEXIST for any name that already exists, of any kind:
 * a directory, a regular file, a symbolic link (not followed: a link to
 * a directory, to a file, a dangling link or a link loop), ".", "..",
 * "/", and the same names with a trailing slash. Nothing is changed: the
 * existing object keeps its type and mode, and a dangling link's target
 * is not created. */
#include "a_sys.h"

#define D "/tmp/c_mkdir_eexist"

static i64 mk(const char *p, i64 m) { return syscall2(SYS_mkdir, p, m); }
static i64 mode_of(const char *p) {
    struct a_stat st;
    i64 r = a_lstat_path(p, &st);
    return r < 0 ? r : (i64)st.st_mode;
}

int test_main(void) {
    expect("setup", a_mkdir(D, 0755), 0);
    expect("setup_dir", a_mkdir(D "/d", 0711), 0);
    expect("setup_file", a_make_file(D "/f", "data", 0600), 0);
    expect("setup_link_dir", syscall2(SYS_symlink, "d", D "/ld"), 0);
    expect("setup_link_file", syscall2(SYS_symlink, D "/f", D "/lf"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "target", D "/dangle"), 0);
    expect("setup_loop1", syscall2(SYS_symlink, "loop2", D "/loop1"), 0);
    expect("setup_loop2", syscall2(SYS_symlink, "loop1", D "/loop2"), 0);

    expect("dir", mk(D "/d", 0700), -EEXIST);
    expect("dir_slash", mk(D "/d/", 0700), -EEXIST);
    expect("dir_mode_unchanged", mode_of(D "/d"), S_IFDIR | 0711);
    expect("file", mk(D "/f", 0700), -EEXIST);
    expect("file_slash", mk(D "/f/", 0700), -EEXIST);
    expect("file_slash_slash", mk(D "/f//", 0700), -EEXIST);
    expect("file_unchanged", mode_of(D "/f"), S_IFREG | 0600);
    expect("link_to_dir", mk(D "/ld", 0700), -EEXIST);
    expect("link_to_dir_slash", mk(D "/ld/", 0700), -EEXIST);
    expect("link_to_file", mk(D "/lf", 0700), -EEXIST);
    expect("link_to_file_slash", mk(D "/lf/", 0700), -EEXIST);
    expect("dangling", mk(D "/dangle", 0700), -EEXIST);
    expect("dangling_slash", mk(D "/dangle/", 0700), -EEXIST);
    expect("dangling_still_link", mode_of(D "/dangle") & S_IFMT, S_IFLNK);
    expect("dangling_target_not_made", mode_of(D "/target"), -ENOENT);
    expect("loop", mk(D "/loop1", 0700), -EEXIST);
    expect("loop_slash", mk(D "/loop1/", 0700), -EEXIST);
    expect("dot", mk(D "/.", 0700), -EEXIST);
    expect("dotdot", mk(D "/..", 0700), -EEXIST);
    expect("sub_dot", mk(D "/d/.", 0700), -EEXIST);
    expect("sub_dotdot", mk(D "/d/..", 0700), -EEXIST);
    expect("root", mk("/", 0700), -EEXIST);
    expect("root_dotdot", mk("/..", 0700), -EEXIST);
    expect("dotdot_above_root", mk("/../tmp", 0700), -EEXIST);
    expect("tmp", mk("/tmp", 0700), -EEXIST);
    expect("tmp_slashes", mk("//tmp//", 0700), -EEXIST);
    expect("self_dir", mk(D, 0700), -EEXIST);
    expect("self_via_dotdot", mk(D "/d/../d", 0700), -EEXIST);

    expect("cleanup_loop2", a_unlink(D "/loop2"), 0);
    expect("cleanup_loop1", a_unlink(D "/loop1"), 0);
    expect("cleanup_dangle", a_unlink(D "/dangle"), 0);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_d", a_rmdir(D "/d"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
