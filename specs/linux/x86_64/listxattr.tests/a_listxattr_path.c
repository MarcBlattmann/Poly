/* listxattr: path resolution. Absolute and relative paths, ".", a trailing
 * slash on a directory and symbolic links (followed: the target's list)
 * work. Errors: missing file or component and the empty path ENOENT, a
 * file used as a directory ENOTDIR, a dangling link ENOENT, a link loop
 * ELOOP, a 256-byte component or a 4096-byte path ENAMETOOLONG (4095
 * bytes resolve), an unreadable path pointer EFAULT, no search permission
 * on a directory on the way EACCES. Path errors come before any use of the
 * buffer or size. All capabilities are dropped first. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_listxattr 194
#define ENAMETOOLONG 36
#define ELOOP 40
#define D "/tmp/a_listxattr_path"
#define BAD ((void *)8)

/* Drop every capability, so root fails the permission checks like any
 * other user (and the results do not depend on who runs the test). */
static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static char buf[256];
static char path[4200];

static i64 list(const char *p) { return syscall3(SYS_listxattr, p, buf, sizeof buf); }

int test_main(void) {
    expect("setup_drop_caps", drop_caps(), 0);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/sub/f", "", 0644), 0);
    expect("setup_set_file", syscall5(SYS_setxattr, D "/sub/f", "user.file", "", 0, 0), 0);
    expect("setup_set_dir", syscall5(SYS_setxattr, D "/sub", "user.dir", "", 0, 0), 0);
    expect("setup_link", syscall2(SYS_symlink, "sub/f", D "/lf"), 0);
    expect("setup_link_dir", syscall2(SYS_symlink, "sub", D "/ld"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nothing", D "/dang"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("absolute", list(D "/sub/f"), 10);
    check("absolute_bytes", mem_eq(buf, "user.file\0", 10), 0);
    expect("dir", list(D "/sub"), 9);
    expect("dir_slash", list(D "/sub/"), 9);
    expect("dir_dot", list(D "/sub/."), 9);
    expect("dotdot", list(D "/sub/../sub/f"), 10);
    expect("follow_link", list(D "/lf"), 10);
    check("follow_bytes", mem_eq(buf, "user.file\0", 10), 0);
    expect("follow_link_dir", list(D "/ld"), 9);
    expect("through_link_dir", list(D "/ld/f"), 10);
    expect("setup_chdir", syscall1(SYS_chdir, D), 0);
    expect("relative", list("sub/f"), 10);
    expect("relative_dot", list("."), 0);
    expect("setup_chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("missing", list(D "/nope"), -ENOENT);
    expect("missing_component", list(D "/nope/f"), -ENOENT);
    expect("missing_bad_buffer", syscall3(SYS_listxattr, D "/nope", BAD, 100), -ENOENT);
    expect("missing_size0", syscall3(SYS_listxattr, D "/nope", 0, 0), -ENOENT);
    expect("empty_path", list(""), -ENOENT);
    expect("null_path", list(0), -EFAULT);
    expect("bad_path", list(BAD), -EFAULT);
    expect("file_as_dir", list(D "/sub/f/x"), -ENOTDIR);
    expect("file_slash", list(D "/sub/f/"), -ENOTDIR);
    expect("dangling", list(D "/dang"), -ENOENT);
    expect("loop", list(D "/loop"), -ELOOP);
    char *q = str_cpy(path, D "/");
    for (int i = 0; i < 256; i++) *q++ = 'n';
    *q = 0;
    expect("component_256", list(path), -ENAMETOOLONG);
    path[str_len(D "/") + 255] = 0;
    expect("component_255_missing", list(path), -ENOENT);
    for (int i = 0; i < 4095; i++) path[i] = (i % 2) ? '.' : '/';
    path[4095] = 0;
    expect("path_4095", list(path), 0);
    path[4095] = '/';
    path[4096] = 0;
    expect("path_4096", list(path), -ENAMETOOLONG);
    expect("setup_nosearch", syscall2(SYS_chmod, D "/sub", 0644), 0);
    expect("no_search", list(D "/sub/f"), -EACCES);
    expect("no_search_bad_buffer", syscall3(SYS_listxattr, D "/sub/f", BAD, 100), -EACCES);
    expect("no_search_dir_itself", list(D "/sub"), 9);
    expect("setup_search", syscall2(SYS_chmod, D "/sub", 0755), 0);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_ld", a_unlink(D "/ld"), 0);
    expect("cleanup_dang", a_unlink(D "/dang"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_f", a_unlink(D "/sub/f"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
