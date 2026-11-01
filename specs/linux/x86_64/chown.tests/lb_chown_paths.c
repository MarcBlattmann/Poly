/* chown(2): path errors and their order; a dangling link is ENOENT
 * (chown follows links); search permission on the way (root passes). */
#include "a_sys.h"

#define D "/tmp/lb_chown_paths"

static i64 chown_(const char *p, i64 u, i64 g) { return syscall3(SYS_chown, p, u, g); }

static char buf[8192];

int test_main(void) {
    int root = syscall0(SYS_getuid) == 0;
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "x", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_inner", a_make_file(D "/sub/g", "x", 0644), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "missing", D "/dangling"), 0);
    expect("setup_loop1", syscall2(SYS_symlink, "loop2", D "/loop1"), 0);
    expect("setup_loop2", syscall2(SYS_symlink, "loop1", D "/loop2"), 0);

    expect("missing_enoent", chown_(D "/nope", -1, -1), -ENOENT);
    expect("missing_dir_enoent", chown_(D "/nope/f", -1, -1), -ENOENT);
    expect("empty_enoent", chown_("", -1, -1), -ENOENT);
    expect("dangling_enoent", chown_(D "/dangling", -1, -1), -ENOENT);
    expect("loop_eloop", chown_(D "/loop1", -1, -1), -ELOOP);
    expect("file_as_dir_enotdir", chown_(D "/f/x", -1, -1), -ENOTDIR);
    expect("file_slash_enotdir", chown_(D "/f/", -1, -1), -ENOTDIR);
    expect("null_efault", chown_((const char *)0, -1, -1), -EFAULT);
    expect("addr1_efault", chown_((const char *)1, -1, -1), -EFAULT);
    char *p = str_cpy(buf, D "/");
    for (int i = 0; i < 256; i++) p[i] = 'c';
    p[256] = 0;
    expect("name_256_enametoolong", chown_(buf, -1, -1), -ENAMETOOLONG);
    p[255] = 0;
    expect("name_255_enoent", chown_(buf, -1, -1), -ENOENT);
    for (int i = 0; i < 4096; i++) buf[i] = '/';
    buf[4096] = 0;
    expect("path_4096_enametoolong", chown_(buf, -1, -1), -ENAMETOOLONG);
    p = str_cpy(buf, D "/f/");
    for (int i = 0; i < 256; i++) p[i] = 'c';
    p[256] = 0;
    expect("order_notdir_before_long", chown_(buf, -1, -1), -ENOTDIR);

    /* relative paths */
    expect("chdir", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative", chown_("g", -1, -1), 0);
    expect("relative_dotdot", chown_("../f", -1, -1), 0);
    expect("relative_missing", chown_("f", -1, -1), -ENOENT);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);

    /* search permission */
    expect("sub_0600", syscall2(SYS_chmod, D "/sub", 0600), 0);
    expect("through_closed", chown_(D "/sub/g", -1, -1), root ? 0 : -EACCES);
    expect("closed_missing", chown_(D "/sub/nope", -1, -1), root ? -ENOENT : -EACCES);
    expect("closed_itself", chown_(D "/sub", -1, -1), 0);
    expect("sub_0755", syscall2(SYS_chmod, D "/sub", 0755), 0);

    expect("cleanup_loop2", a_unlink(D "/loop2"), 0);
    expect("cleanup_loop1", a_unlink(D "/loop1"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_inner", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
