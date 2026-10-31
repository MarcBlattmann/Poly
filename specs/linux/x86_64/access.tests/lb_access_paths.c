/* access(2): path resolution. Symbolic links are followed (a dangling one
 * is ENOENT, a loop ELOOP), relative paths start at the working directory,
 * and the usual path errors with their order. */
#include "a_sys.h"

#define D "/tmp/lb_access_paths"

static i64 acc(const char *p, i64 mode) { return syscall2(SYS_access, p, mode); }
static i64 symlink_(const char *target, const char *path) { return syscall2(SYS_symlink, target, path); }
static i64 chmod_(const char *p, i64 mode) { return syscall2(SYS_chmod, p, mode); }

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0600), 0);
    expect("setup_exe", a_make_file(D "/exe", "x", 0700), 0);
    expect("setup_link_f", symlink_("f", D "/lf"), 0);
    expect("setup_link_exe", symlink_(D "/exe", D "/lexe"), 0);
    expect("setup_link_sub", symlink_("sub", D "/lsub"), 0);
    expect("setup_dangling", symlink_("nothing", D "/dangling"), 0);
    expect("setup_loop_a", symlink_("loop_b", D "/loop_a"), 0);
    expect("setup_loop_b", symlink_("loop_a", D "/loop_b"), 0);
    expect("setup_self", symlink_("self", D "/self"), 0);

    /* The link is followed: the target's mode decides, not the link's 0777. */
    expect("link_to_file_fok", acc(D "/lf", F_OK), 0);
    expect("link_to_file_x", acc(D "/lf", X_OK), -EACCES);
    expect("link_to_exe_x", acc(D "/lexe", X_OK), 0);
    expect("chmod_f_000", chmod_(D "/f", 0), 0);
    expect("link_to_000_r", acc(D "/lf", R_OK), root ? 0 : -EACCES);
    expect("link_to_dir_x", acc(D "/lsub", X_OK), 0);
    expect("link_to_dir_slash", acc(D "/lsub/", F_OK), 0);
    expect("through_link_dir", acc(D "/lsub/../f", F_OK), 0);
    expect("dangling_enoent", acc(D "/dangling", F_OK), -ENOENT);
    expect("dangling_slash_enoent", acc(D "/dangling/", F_OK), -ENOENT);
    expect("loop_eloop", acc(D "/loop_a", F_OK), -ELOOP);
    expect("self_eloop", acc(D "/self", F_OK), -ELOOP);
    expect("loop_in_middle_eloop", acc(D "/loop_a/x", F_OK), -ELOOP);

    /* Missing names, non-directories, trailing slashes. */
    expect("missing_enoent", acc(D "/nope", F_OK), -ENOENT);
    expect("missing_dir_enoent", acc(D "/nope/f", F_OK), -ENOENT);
    expect("missing_then_dotdot_enoent", acc(D "/nope/../f", F_OK), -ENOENT);
    expect("empty_enoent", acc("", F_OK), -ENOENT);
    expect("file_as_dir_enotdir", acc(D "/f/x", F_OK), -ENOTDIR);
    expect("file_slash_enotdir", acc(D "/f/", F_OK), -ENOTDIR);
    expect("file_dot_enotdir", acc(D "/f/.", F_OK), -ENOTDIR);
    expect("link_file_slash_enotdir", acc(D "/lf/", F_OK), -ENOTDIR);
    expect("dir_slashes", acc(D "//sub///", R_OK | X_OK), 0);
    expect("dot", acc(D "/sub/.", F_OK), 0);
    expect("dotdot", acc(D "/sub/..", F_OK), 0);
    expect("root", acc("/", F_OK), 0);
    expect("root_dotdot", acc("/..", X_OK), 0);

    /* Relative paths start at the working directory ("/" at start). */
    expect("relative_from_root", acc("tmp/lb_access_paths/f", F_OK), 0);
    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative_dotdot", acc("../exe", X_OK), 0);
    expect("relative_dot", acc(".", F_OK), 0);
    expect("relative_missing", acc("f", F_OK), -ENOENT);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);

    /* Lengths: a 255-byte name is fine (missing), 256 is too long; a path of
     * 4095 bytes plus NUL resolves, 4096 plus NUL is too long. */
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 255; i++) p[i] = 'n';
    p[255] = 0;
    expect("name_255_enoent", acc(buf, F_OK), -ENOENT);
    p[255] = 'n';
    p[256] = 0;
    expect("name_256_enametoolong", acc(buf, F_OK), -ENAMETOOLONG);
    for (int i = 0; i < 4095; i++) buf[i] = (i % 2) ? '.' : '/';
    buf[4095] = 0;
    expect("path_4095_ok", acc(buf, F_OK), 0);
    buf[4095] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", acc(buf, F_OK), -ENAMETOOLONG);

    /* Order: the first failing component decides. */
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 'n';
    p[256] = 0;
    expect("order_missing_before_long", acc(buf, F_OK), -ENOENT);
    p = str_cpy(buf, D "/f/");
    for (int i = 0; i < 256; i++) p[i] = 'n';
    p[256] = 0;
    expect("order_notdir_before_long", acc(buf, F_OK), -ENOTDIR);
    p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'n';
    str_cpy(p + 256, "/nope");
    expect("order_long_before_missing", acc(buf, F_OK), -ENAMETOOLONG);

    /* Unreadable path pointers. */
    expect("null_efault", acc((const char *)0, F_OK), -EFAULT);
    expect("addr1_efault", acc((const char *)1, F_OK), -EFAULT);
    expect("kernel_addr_efault", acc((const char *)0xffff800000000000ul, F_OK), -EFAULT);

    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_loop_b", a_unlink(D "/loop_b"), 0);
    expect("cleanup_loop_a", a_unlink(D "/loop_a"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_lsub", a_unlink(D "/lsub"), 0);
    expect("cleanup_lexe", a_unlink(D "/lexe"), 0);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_exe", a_unlink(D "/exe"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
