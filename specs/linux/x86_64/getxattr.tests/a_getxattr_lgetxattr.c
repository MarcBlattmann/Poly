/* lgetxattr (192) does not follow a final symbolic link: user.* on the
 * link itself is ENODATA (live, dangling or looping link) while getxattr
 * on the same path reads the target (or fails with ENOENT / ELOOP). Links
 * in earlier components are followed, and so is a link followed by a
 * trailing slash. On files and directories lgetxattr reads like
 * getxattr, with the same size and name rules. */
#include "a_sys.h"

#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_getxattr_lgetxattr"

static char buf[64];

static i64 get(const char *p, const char *name) { return syscall4(SYS_getxattr, p, name, buf, sizeof buf); }
static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_f", syscall5(SYS_setxattr, D "/sub/f", "user.a", "file", 4, 0), 0);
    expect("setup_set_sub", syscall5(SYS_setxattr, D "/sub", "user.a", "dir", 3, 0), 0);
    expect("setup_link_f", syscall2(SYS_symlink, "sub/f", D "/lf"), 0);
    expect("setup_link_sub", syscall2(SYS_symlink, "sub", D "/ls"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nope", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("get_follows", get(D "/lf", "user.a"), 4);
    expect("lget_link_enodata", lget(D "/lf", "user.a"), -ENODATA);
    expect("lget_link_size0", syscall4(SYS_lgetxattr, D "/lf", "user.a", 0, 0), -ENODATA);
    expect("lget_dir_link_enodata", lget(D "/ls", "user.a"), -ENODATA);
    expect("lget_dir_link_slash", lget(D "/ls/", "user.a"), 3);
    check("dir_link_slash_value", mem_eq(buf, "dir", 3), 0);
    expect("lget_link_in_middle", lget(D "/ls/f", "user.a"), 4);
    check("middle_value", mem_eq(buf, "file", 4), 0);
    expect("get_dangling_enoent", get(D "/dangling", "user.a"), -ENOENT);
    expect("lget_dangling_enodata", lget(D "/dangling", "user.a"), -ENODATA);
    expect("get_loop_eloop", get(D "/loop", "user.a"), -ELOOP);
    expect("lget_loop_enodata", lget(D "/loop", "user.a"), -ENODATA);
    expect("lget_file", lget(D "/sub/f", "user.a"), 4);
    expect("lget_dir", lget(D "/sub", "user.a"), 3);
    expect("lget_file_size0", syscall4(SYS_lgetxattr, D "/sub/f", "user.a", 0, 0), 4);
    expect("lget_file_erange", syscall4(SYS_lgetxattr, D "/sub/f", "user.a", buf, 3), -ERANGE);
    expect("lget_missing_name", lget(D "/sub/f", "user.b"), -ENODATA);
    expect("lget_empty_name", lget(D "/sub/f", ""), -ERANGE);
    expect("lget_unknown_prefix", lget(D "/sub/f", "foo.a"), -EOPNOTSUPP);
    expect("lget_missing_path", lget(D "/nope", "user.a"), -ENOENT);
    expect("lget_dangling_middle", lget(D "/dangling/x", "user.a"), -ENOENT);
    expect("lget_file_link_slash", lget(D "/lf/", "user.a"), -ENOTDIR);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_ls", a_unlink(D "/ls"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
