/* removexattr path handling, with all capabilities dropped first: a
 * missing file or component and an empty path are ENOENT; NULL or
 * unmapped path EFAULT; a file used as a directory or with a trailing
 * slash ENOTDIR; a directory with a trailing slash or "/." works; a
 * component of 256 bytes ENAMETOOLONG (255 is just ENOENT); a path of 4096
 * bytes before its NUL ENAMETOOLONG, 4095 bytes resolve; relative paths
 * start at the working directory; a directory without search permission
 * on the way is EACCES. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_getxattr 191
#define SYS_removexattr 197
#define ENODATA 61
#define D "/tmp/a_removexattr_path"

static char lp[4200];
static char buf[16];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 rm(const void *p, const char *name) { return syscall2(SYS_removexattr, p, name); }
static i64 set(const char *p, const char *name) { return syscall5(SYS_setxattr, p, name, "v", 1, 0); }

/* D, then slashes, then "/f": exactly len bytes before the NUL. */
static const char *padded(int len) {
    str_cpy(lp, D);
    int l = str_len(lp);
    while (l < len - 2) lp[l++] = '/';
    lp[l++] = '/';
    lp[l++] = 'f';
    lp[l] = 0;
    return lp;
}

int test_main(void) {
    expect("drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("missing_enoent", rm(D "/nope", "user.a"), -ENOENT);
    expect("missing_dir_enoent", rm(D "/nope/f", "user.a"), -ENOENT);
    expect("empty_enoent", rm("", "user.a"), -ENOENT);
    expect("null_efault", rm((const void *)0, "user.a"), -EFAULT);
    expect("unmapped_efault", rm((const void *)8, "user.a"), -EFAULT);
    expect("file_as_dir_enotdir", rm(D "/f/x", "user.a"), -ENOTDIR);
    expect("file_slash_enotdir", rm(D "/f/", "user.a"), -ENOTDIR);
    expect("set_sub", set(D "/sub", "user.a"), 0);
    expect("dir_slash_ok", rm(D "/sub/", "user.a"), 0);
    expect("set_sub2", set(D "/sub", "user.a"), 0);
    expect("dir_dot_ok", rm(D "/sub/.", "user.a"), 0);
    expect("set_sub3", set(D "/sub", "user.a"), 0);
    expect("dotdot_ok", rm(D "/sub/../sub", "user.a"), 0);
    str_cpy(lp, D "/");
    int l = str_len(lp);
    for (int i = 0; i < 256; i++) lp[l + i] = 'c';
    lp[l + 256] = 0;
    expect("component_256_enametoolong", rm(lp, "user.a"), -ENAMETOOLONG);
    lp[l + 255] = 0;
    expect("component_255_enoent", rm(lp, "user.a"), -ENOENT);
    expect("set_f", set(D "/f", "user.p"), 0);
    expect("path_4096_enametoolong", rm(padded(4096), "user.p"), -ENAMETOOLONG);
    expect("path_4095_ok", rm(padded(4095), "user.p"), 0);
    expect("path_4095_gone", syscall4(SYS_getxattr, D "/f", "user.p", buf, sizeof buf), -ENODATA);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("set_rel", set("f", "user.r"), 0);
    expect("relative_ok", rm("f", "user.r"), 0);
    expect("relative_dot_slash", rm("./f", "user.r"), -ENODATA);
    expect("relative_missing", rm("nope", "user.r"), -ENOENT);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("setup_nox_f", a_make_file(D "/sub/g", "", 0644), 0);
    expect("set_nox_f", set(D "/sub/g", "user.a"), 0);
    expect("setup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0600), 0);
    expect("no_search_eacces", rm(D "/sub/g", "user.a"), -EACCES);
    expect("cleanup_nox_mode", syscall3(SYS_fchmodat, AT_FDCWD, D "/sub", 0755), 0);
    expect("nox_attr_stays", syscall4(SYS_getxattr, D "/sub/g", "user.a", buf, sizeof buf), 1);
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
