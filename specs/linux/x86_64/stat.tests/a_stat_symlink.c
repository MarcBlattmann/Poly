/* stat: symbolic links are followed, in the last component and in the
 * middle of a path, through chains; dangling links and loops fail. */
#include "a_sys.h"

#define D "/tmp/a_stat_symlink"

static i64 st(const char *p, struct a_stat *s) { return syscall2(SYS_stat, p, s); }
static i64 lst(const char *p, struct a_stat *s) { return syscall2(SYS_lstat, p, s); }
static i64 sl(const char *target, const char *path) { return syscall3(SYS_symlinkat, target, AT_FDCWD, path); }

int test_main(void) {
    struct a_stat f, s, l;
    const char *names[] = {D "/l_file", D "/l_rel", D "/l_chain", D "/l_dir", D "/l_dangling", D "/l_self",
                           D "/l_a", D "/l_b", D "/sub/g", D "/f"};
    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) a_unlink(names[i]);
    a_rmdir(D "/sub");
    a_rmdir(D);

    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "0123456789", 0600), 0);
    expect("setup_sub_file", a_make_file(D "/sub/g", "abc", 0644), 0);
    st(D "/f", &f);

    expect("symlink_abs", sl(D "/f", D "/l_file"), 0);
    expect("stat_follows", st(D "/l_file", &s), 0);
    expect("followed_mode", s.st_mode, S_IFREG | 0600);
    expect("followed_size", s.st_size, 10);
    check("followed_same_inode", s.st_ino == f.st_ino && s.st_dev == f.st_dev, 0);
    expect("lstat_does_not_follow", lst(D "/l_file", &l), 0);
    expect("lstat_is_link", l.st_mode & S_IFMT, S_IFLNK);
    check("link_own_inode", l.st_ino != f.st_ino, 0);

    expect("symlink_relative", sl("f", D "/l_rel"), 0);
    expect("relative_resolves_from_link_dir", st(D "/l_rel", &s), 0);
    check("relative_same_inode", s.st_ino == f.st_ino, 0);

    expect("symlink_chain", sl("l_rel", D "/l_chain"), 0);
    expect("chain_followed", st(D "/l_chain", &s), 0);
    check("chain_same_inode", s.st_ino == f.st_ino, 0);

    expect("symlink_dir", sl("sub", D "/l_dir"), 0);
    expect("link_to_dir", st(D "/l_dir", &s), 0);
    expect("link_to_dir_is_dir", s.st_mode & S_IFMT, S_IFDIR);
    expect("link_to_dir_slash", st(D "/l_dir/", &s), 0);
    expect("link_in_middle", st(D "/l_dir/g", &s), 0);
    expect("link_in_middle_size", s.st_size, 3);
    expect("link_then_dotdot", st(D "/l_dir/../f", &s), 0);
    check("dotdot_after_link_is_physical_parent", s.st_ino == f.st_ino, 0);

    expect("symlink_dangling", sl(D "/missing", D "/l_dangling"), 0);
    expect("dangling_enoent", st(D "/l_dangling", &s), -ENOENT);
    expect("dangling_lstat_ok", lst(D "/l_dangling", &s), 0);
    expect("dangling_slash_enoent", st(D "/l_dangling/", &s), -ENOENT);

    expect("link_to_file_then_slash", st(D "/l_file/", &s), -ENOTDIR);
    expect("link_to_file_as_dir", st(D "/l_file/x", &s), -ENOTDIR);

    expect("symlink_self", sl("l_self", D "/l_self"), 0);
    expect("self_loop_eloop", st(D "/l_self", &s), -ELOOP);
    expect("self_loop_middle_eloop", st(D "/l_self/x", &s), -ELOOP);
    expect("symlink_a", sl("l_b", D "/l_a"), 0);
    expect("symlink_b", sl("l_a", D "/l_b"), 0);
    expect("mutual_loop_eloop", st(D "/l_a", &s), -ELOOP);
    expect("loop_lstat_ok", lst(D "/l_a", &s), 0);

    for (unsigned i = 0; i < sizeof names / sizeof names[0]; i++) a_unlink(names[i]);
    expect("cleanup_rmdir_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
