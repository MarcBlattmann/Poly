/* unlink on symbolic links: the link itself goes, never its target;
 * dangling and self-referencing links can be removed; links in earlier
 * components are followed (and a loop there is ELOOP). */
#include "a_sys.h"

#define D "/tmp/c_unlink_symlinks"

static i64 u(const char *p) { return syscall1(SYS_unlink, p); }

int test_main(void) {
    struct a_stat st;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_target", a_make_file(D "/target", "t", 0644), 0);
    expect("setup_inner", a_make_file(D "/sub/inner", "", 0644), 0);
    expect("link_to_file", syscall2(SYS_symlink, "target", D "/lf"), 0);
    expect("link_to_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("link_dangling", syscall2(SYS_symlink, "missing", D "/dang"), 0);
    expect("link_self", syscall2(SYS_symlink, "self", D "/self"), 0);
    expect("link_abs", syscall2(SYS_symlink, D "/target", D "/labs"), 0);
    expect("unlink_lf", u(D "/lf"), 0);
    expect("lf_gone", a_lstat_path(D "/lf", &st), -ENOENT);
    expect("target_kept", a_stat_path(D "/target", &st), 0);
    expect("target_nlink_1", st.st_nlink, 1);
    expect("unlink_labs", u(D "/labs"), 0);
    expect("target_kept_abs", a_stat_path(D "/target", &st), 0);
    expect("unlink_ld", u(D "/ld"), 0);
    expect("sub_kept", a_stat_path(D "/sub/inner", &st), 0);
    expect("unlink_dangling", u(D "/dang"), 0);
    expect("dang_gone", a_lstat_path(D "/dang", &st), -ENOENT);
    expect("unlink_self_loop", u(D "/self"), 0);
    expect("self_gone", a_lstat_path(D "/self", &st), -ENOENT);
    /* Links before the last component are followed. */
    expect("relink_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("unlink_through_dir_link", u(D "/ld/inner"), 0);
    expect("inner_gone", a_stat_path(D "/sub/inner", &st), -ENOENT);
    expect("ld_still_there", a_lstat_path(D "/ld", &st), 0);
    expect("missing_through_link_enoent", u(D "/ld/inner"), -ENOENT);
    expect("loop_a", syscall2(SYS_symlink, "lb", D "/la"), 0);
    expect("loop_b", syscall2(SYS_symlink, "la", D "/lb"), 0);
    expect("through_loop_eloop", u(D "/la/x"), -ELOOP);
    expect("dangling_dir_component_enoent", syscall2(SYS_symlink, "gone", D "/lg") == 0 ? u(D "/lg/x") : -1,
           -ENOENT);
    expect("link_to_file_component_enotdir", syscall2(SYS_symlink, "target", D "/lt") == 0 ? u(D "/lt/x") : -1,
           -ENOTDIR);
    expect("unlink_la", u(D "/la"), 0);
    expect("unlink_lb", u(D "/lb"), 0);
    expect("unlink_lg", u(D "/lg"), 0);
    expect("unlink_lt", u(D "/lt"), 0);
    expect("cleanup_ld", u(D "/ld"), 0);
    expect("cleanup_target", u(D "/target"), 0);
    expect("cleanup_sub", syscall1(SYS_rmdir, D "/sub"), 0);
    expect("cleanup_dir", syscall1(SYS_rmdir, D), 0);
    return finish();
}
