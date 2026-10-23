/* Only the final component is left unfollowed: symbolic links in earlier
 * components are followed, and so is a link followed by a trailing slash
 * or by "/." (the attribute then lands on the directory it names). A
 * trailing slash after a link to a file is ENOTDIR, after a dangling link
 * ENOENT, after a looping link ELOOP. */
#include "a_sys.h"

#define SYS_lsetxattr 189
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lsetxattr_follow"

static char buf[64];

static i64 lset(const char *p, const char *name) { return syscall5(SYS_lsetxattr, p, name, "v", 1, 0); }

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_flink", a_make_file(D "/f", "", 0644), 0);
    expect("setup_lf", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/lf"), 0);
    expect("setup_ls", syscall3(SYS_symlinkat, "sub", AT_FDCWD, D "/ls"), 0);
    expect("setup_lls", syscall3(SYS_symlinkat, "ls", AT_FDCWD, D "/lls"), 0);
    expect("setup_dl", syscall3(SYS_symlinkat, "nope", AT_FDCWD, D "/dl"), 0);
    expect("setup_loop", syscall3(SYS_symlinkat, "loop", AT_FDCWD, D "/loop"), 0);
    expect("setup_self", syscall3(SYS_symlinkat, ".", AT_FDCWD, D "/self"), 0);
    expect("setup_inner", syscall3(SYS_symlinkat, "g", AT_FDCWD, D "/sub/lg"), 0);
    expect("through_dir_link", lset(D "/ls/g", "user.a"), 0);
    expect("through_value", syscall4(SYS_getxattr, D "/sub/g", "user.a", buf, sizeof buf), 1);
    expect("through_chain", lset(D "/lls/g", "user.b"), 0);
    expect("through_chain_value", syscall4(SYS_getxattr, D "/sub/g", "user.b", buf, sizeof buf), 1);
    expect("through_self", lset(D "/self/f", "user.c"), 0);
    expect("through_self_value", syscall4(SYS_getxattr, D "/f", "user.c", buf, sizeof buf), 1);
    expect("final_after_dir_link_eperm", lset(D "/ls/lg", "user.d"), -EPERM);
    expect("final_after_dir_link_untouched", syscall4(SYS_getxattr, D "/sub/g", "user.d", buf, sizeof buf), -ENODATA);
    expect("dirlink_slash", lset(D "/ls/", "user.s"), 0);
    expect("dirlink_slash_value", syscall4(SYS_getxattr, D "/sub", "user.s", buf, sizeof buf), 1);
    expect("chain_slash", lset(D "/lls/", "user.t"), 0);
    expect("chain_slash_value", syscall4(SYS_getxattr, D "/sub", "user.t", buf, sizeof buf), 1);
    expect("dirlink_dot", lset(D "/ls/.", "user.u"), 0);
    expect("dirlink_dot_value", syscall4(SYS_getxattr, D "/sub", "user.u", buf, sizeof buf), 1);
    expect("chain_link_bare", syscall4(SYS_lgetxattr, D "/lls", "user.t", buf, sizeof buf), -ENODATA);
    expect("dirlink_itself_eperm", lset(D "/ls", "user.v"), -EPERM);
    expect("filelink_slash_enotdir", lset(D "/lf/", "user.w"), -ENOTDIR);
    expect("filelink_dot_enotdir", lset(D "/lf/.", "user.w"), -ENOTDIR);
    expect("dangling_slash_enoent", lset(D "/dl/", "user.w"), -ENOENT);
    expect("loop_slash_eloop", lset(D "/loop/", "user.w"), -ELOOP);
    expect("loop_mid_eloop", lset(D "/loop/x", "user.w"), -ELOOP);
    expect("dangling_mid_enoent", lset(D "/dl/x", "user.w"), -ENOENT);
    expect("file_untouched", syscall4(SYS_getxattr, D "/f", "user.w", buf, sizeof buf), -ENODATA);
    a_unlink(D "/sub/lg");
    a_unlink(D "/lf");
    a_unlink(D "/ls");
    a_unlink(D "/lls");
    a_unlink(D "/dl");
    a_unlink(D "/loop");
    a_unlink(D "/self");
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
