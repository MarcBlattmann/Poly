/* chmod(2): path errors and their order, relative paths, search permission
 * on the directories walked (root passes it), and an error leaves the mode
 * alone. */
#include "a_sys.h"

#define D "/tmp/lb_chmod_paths"

static i64 chmod_(const char *p, i64 mode) { return syscall2(SYS_chmod, p, mode); }
static u32 mode_of(const char *p) {
    struct a_stat st;
    if (a_stat_path(p, &st)) return 0xffffffff;
    return st.st_mode;
}

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner", a_make_file(D "/sub/g", "x", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "missing", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);

    expect("missing_enoent", chmod_(D "/nope", 0600), -ENOENT);
    expect("missing_dir_enoent", chmod_(D "/nope/f", 0600), -ENOENT);
    expect("empty_enoent", chmod_("", 0600), -ENOENT);
    expect("dangling_enoent", chmod_(D "/dangling", 0600), -ENOENT);
    expect("loop_eloop", chmod_(D "/loop", 0600), -ELOOP);
    expect("file_as_dir_enotdir", chmod_(D "/f/x", 0600), -ENOTDIR);
    expect("file_slash_enotdir", chmod_(D "/f/", 0600), -ENOTDIR);
    expect("mode_unchanged_after_errors", mode_of(D "/f"), S_IFREG | 0644);
    expect("null_efault", chmod_((const char *)0, 0600), -EFAULT);
    expect("addr1_efault", chmod_((const char *)1, 0600), -EFAULT);

    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'n';
    p[256] = 0;
    expect("name_256_enametoolong", chmod_(buf, 0600), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255_enoent", chmod_(buf, 0600), -ENOENT);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", chmod_(buf, 0600), -ENAMETOOLONG);
    p = str_cpy(buf, D "/nope/");
    for (int i = 0; i < 256; i++) p[i] = 'n';
    p[256] = 0;
    expect("order_missing_before_long", chmod_(buf, 0600), -ENOENT);

    /* Relative paths. */
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("relative", chmod_("sub/g", 0600), 0);
    expect("relative_mode", mode_of(D "/sub/g"), S_IFREG | 0600);
    expect("relative_dot", chmod_(".", 0750), 0);
    expect("dot_mode", mode_of(D), S_IFDIR | 0750);
    expect("relative_dotdot", chmod_("sub/../f", 0640), 0);
    expect("dotdot_mode", mode_of(D "/f"), S_IFREG | 0640);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    /* Search permission: sub without x. */
    expect("sub_0600", chmod_(D "/sub", 0600), 0);
    expect("through_closed_dir", chmod_(D "/sub/g", 0644), root ? 0 : -EACCES);
    expect("closed_dir_missing", chmod_(D "/sub/nope", 0644), root ? -ENOENT : -EACCES);
    /* the directory itself can still be changed by its owner */
    expect("closed_dir_itself", chmod_(D "/sub", 0700), 0);
    expect("inner_mode", mode_of(D "/sub/g"), S_IFREG | (root ? 0644 : 0600));

    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_inner", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
