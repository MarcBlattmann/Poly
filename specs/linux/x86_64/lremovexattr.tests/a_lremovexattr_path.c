/* lremovexattr path handling: NULL or unmapped path EFAULT; empty path
 * ENOENT; missing file or missing directory on the way ENOENT; a file used
 * as a directory (also with a trailing slash) ENOTDIR; a component of 256
 * bytes ENAMETOOLONG (255 is just looked up: ENOENT); a path of 4096 bytes
 * before its NUL ENAMETOOLONG, 4095 resolves (repeated slashes count but
 * are otherwise harmless); relative paths, ".", ".." work from the working
 * directory; a relative symbolic link as the last component is the link. */
#include "a_sys.h"

#define SYS_chdir 80
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_lremovexattr 198
#define ENODATA 61
#define D "/tmp/a_lremovexattr_path"

static char big[4200], comp[400], buf[8];

static i64 lrm(const void *p, const char *name) { return syscall2(SYS_lremovexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }

/* D, then slashes, then "/f", exactly len bytes long */
static const char *slashed(int len) {
    str_cpy(big, D);
    int n = str_len(big);
    while (n < len - 2) big[n++] = '/';
    big[n++] = '/';
    big[n++] = 'f';
    big[n] = 0;
    return big;
}

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_link", syscall3(SYS_symlinkat, "f", AT_FDCWD, D "/sub/l"), 0);
    expect("null_path_efault", lrm((const void *)0, "user.a"), -EFAULT);
    expect("unmapped_path_efault", lrm((const void *)8, "user.a"), -EFAULT);
    expect("empty_path_enoent", lrm("", "user.a"), -ENOENT);
    expect("missing_enoent", lrm(D "/nope", "user.a"), -ENOENT);
    expect("missing_dir_enoent", lrm(D "/nope/f", "user.a"), -ENOENT);
    expect("file_as_dir_enotdir", lrm(D "/f/x", "user.a"), -ENOTDIR);
    expect("file_trailing_slash_enotdir", lrm(D "/f/", "user.a"), -ENOTDIR);
    str_cpy(comp, D "/");
    int n = str_len(comp);
    for (int i = 0; i < 256; i++) comp[n + i] = 'c';
    comp[n + 256] = 0;
    expect("component_256_enametoolong", lrm(comp, "user.a"), -ENAMETOOLONG);
    comp[n + 255] = 0;
    expect("component_255_enoent", lrm(comp, "user.a"), -ENOENT);
    expect("set_a", set(D "/f", "user.a"), 0);
    expect("path_4096_enametoolong", lrm(slashed(4096), "user.a"), -ENAMETOOLONG);
    expect("path_4095_ok", lrm(slashed(4095), "user.a"), 0);
    expect("path_4095_again", lrm(slashed(4095), "user.a"), -ENODATA);
    expect("set_b", set(D "/f", "user.b"), 0);
    expect("chdir_sub", syscall1(SYS_chdir, D "/sub"), 0);
    expect("relative_link_eperm", lrm("l", "user.b"), -EPERM);
    expect("dotdot_ok", lrm("../f", "user.b"), 0);
    expect("dotdot_again", lrm("./../f", "user.b"), -ENODATA);
    expect("set_sub", set(D "/sub", "user.s"), 0);
    expect("dot_ok", lrm(".", "user.s"), 0);
    expect("dot_again", lrm(".", "user.s"), -ENODATA);
    expect("relative_missing", lrm("nope", "user.s"), -ENOENT);
    expect("chdir_root", syscall1(SYS_chdir, "/"), 0);
    expect("getxattr_b_gone", syscall4(SYS_getxattr, D "/f", "user.b", buf, sizeof buf), -ENODATA);
    a_unlink(D "/sub/l");
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
