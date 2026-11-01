/* renameat(2) is renameat2 with flags 0: replace semantics for files and
 * empty directories through descriptors, hard links of the same file left
 * alone, symbolic links not followed, descriptors of the removed
 * directory still valid. */
#include "a_sys.h"

#define D "/tmp/lb_renameat_replace"

static i64 renameat_(i64 ofd, const char *a, i64 nfd, const char *b) { return syscall4(SYS_renameat, ofd, a, nfd, b); }
static i64 ino_at(i64 fd, const char *p) {
    struct a_stat st;
    if (syscall4(SYS_newfstatat, fd, p, &st, AT_SYMLINK_NOFOLLOW)) return -1;
    return (i64)st.st_ino;
}

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    i64 d = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("open_d", d >= 0, d);
    expect("setup_f", a_make_file(D "/f", "f", 0644), 0);
    expect("setup_g", a_make_file(D "/g", "g", 0644), 0);
    i64 ino_f = ino_at(d, "f");
    expect("replace_file", renameat_(d, "f", d, "g"), 0);
    expect("g_is_f", ino_at(d, "g"), ino_f);
    expect("f_gone", ino_at(d, "f"), -1);
    expect("hardlink", syscall5(SYS_linkat, d, "g", d, "h", 0), 0);
    expect("onto_hardlink", renameat_(d, "g", d, "h"), 0);
    expect("both_remain", ino_at(d, "g") == ino_f && ino_at(d, "h") == ino_f, 1);
    expect("unlink_h", syscall3(SYS_unlinkat, d, "h", 0), 0);

    expect("setup_l", syscall3(SYS_symlinkat, "g", d, "l"), 0);
    i64 ino_l = ino_at(d, "l");
    expect("rename_link", renameat_(d, "l", d, "l2"), 0);
    expect("link_moved", ino_at(d, "l2"), ino_l);
    expect("target_same", ino_at(d, "g"), ino_f);
    expect("link_over_target", renameat_(d, "l2", d, "g"), 0);
    expect("g_is_link_now", syscall4(SYS_newfstatat, d, "g", &st, AT_SYMLINK_NOFOLLOW) == 0 && (st.st_mode & S_IFMT) == S_IFLNK, 1);
    expect("g_loops_to_itself", syscall4(SYS_newfstatat, d, "g", &st, 0), -ELOOP);

    expect("setup_e1", syscall3(SYS_mkdirat, d, "e1", 0755), 0);
    expect("setup_e2", syscall3(SYS_mkdirat, d, "e2", 0755), 0);
    i64 e2 = syscall4(SYS_openat, d, "e2", O_RDONLY | O_DIRECTORY, 0);
    check("open_e2", e2 >= 0, e2);
    i64 ino_e1 = ino_at(d, "e1");
    expect("replace_empty_dir", renameat_(d, "e1", d, "e2/"), 0);
    expect("e2_is_e1", ino_at(d, "e2"), ino_e1);
    expect("old_e2_fd_nlink0", syscall2(SYS_fstat, e2, &st) == 0 && st.st_nlink == 0, 1);
    expect("create_in_old_e2", syscall4(SYS_openat, e2, "x", O_CREAT | O_WRONLY, 0644), -ENOENT);
    expect("close_e2", a_close(e2), 0);
    expect("setup_inner", syscall3(SYS_mkdirat, d, "e2/inner", 0755), 0);
    expect("setup_e3", syscall3(SYS_mkdirat, d, "e3", 0755), 0);
    expect("onto_non_empty", renameat_(d, "e3", d, "e2"), -ENOTEMPTY);
    expect("into_self", renameat_(d, "e2", d, "e2/inner/x"), -EINVAL);

    expect("cleanup_inner", syscall3(SYS_unlinkat, d, "e2/inner", AT_REMOVEDIR), 0);
    expect("cleanup_e2", syscall3(SYS_unlinkat, d, "e2", AT_REMOVEDIR), 0);
    expect("cleanup_e3", syscall3(SYS_unlinkat, d, "e3", AT_REMOVEDIR), 0);
    expect("cleanup_g", syscall3(SYS_unlinkat, d, "g", 0), 0);
    expect("close_d", a_close(d), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
