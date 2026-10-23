/* lgetxattr path walk: symbolic links before the final component are
 * followed ("ls/f" with ls -> sub reads sub/f, "self/f" with self -> "."
 * reads f, chains of links), but a final link reached through followed
 * links is still not followed ("ls/lf" with sub/lf -> ../f: ENODATA). A
 * trailing slash or "/." after a link to a directory follows it and reads
 * the directory; ".." after it goes to the parent of the directory. After
 * a link to a file a trailing slash or ".." is ENOTDIR; after a dangling
 * link ENOENT; after a looping link ELOOP. */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126
#define SYS_setxattr 188
#define SYS_lgetxattr 192
#define ENODATA 61
#define D "/tmp/a_lgetxattr_walk"

static char buf[64];

static i64 drop_caps(void) {
    u32 h[2] = { 0x20080522, 0 }, d[6] = { 0 }, e[6];
    i64 r = syscall2(SYS_capset, h, d);
    if (r < 0) return r;
    r = syscall2(SYS_capget, h, e);
    if (r < 0) return r;
    return (e[0] | e[1] | e[3] | e[4]) ? -1 : 0;
}

static i64 lget(const char *p, const char *name) { return syscall4(SYS_lgetxattr, p, name, buf, sizeof buf); }

int test_main(void) {
    i64 dropped = drop_caps();
    expect("drop_caps", dropped, 0);
    if (dropped != 0) return finish();
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_sub", a_mkdir(D "/sub", 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    expect("setup_subfile", a_make_file(D "/sub/g", "", 0644), 0);
    expect("setup_set_top", syscall5(SYS_setxattr, D, "user.a", "top", 3, 0), 0);
    expect("setup_set_f", syscall5(SYS_setxattr, D "/f", "user.a", "file", 4, 0), 0);
    expect("setup_set_g", syscall5(SYS_setxattr, D "/sub/g", "user.a", "subfile", 7, 0), 0);
    expect("setup_set_sub", syscall5(SYS_setxattr, D "/sub", "user.a", "subdir", 6, 0), 0);
    expect("setup_ls", syscall2(SYS_symlink, "sub", D "/ls"), 0);
    expect("setup_ls2", syscall2(SYS_symlink, "ls", D "/ls2"), 0);
    expect("setup_abs", syscall2(SYS_symlink, D "/sub", D "/labs"), 0);
    expect("setup_self", syscall2(SYS_symlink, ".", D "/self"), 0);
    expect("setup_sub_lf", syscall2(SYS_symlink, "../f", D "/sub/lf"), 0);
    expect("setup_lf", syscall2(SYS_symlink, "f", D "/lf"), 0);
    expect("setup_dangling", syscall2(SYS_symlink, "nope", D "/dangling"), 0);
    expect("setup_loop", syscall2(SYS_symlink, "loop", D "/loop"), 0);
    expect("middle_link", lget(D "/ls/g", "user.a"), 7);
    check("middle_value", mem_eq(buf, "subfile", 7), 0);
    expect("middle_chain", lget(D "/ls2/g", "user.a"), 7);
    expect("middle_absolute", lget(D "/labs/g", "user.a"), 7);
    expect("middle_self", lget(D "/self/f", "user.a"), 4);
    check("self_value", mem_eq(buf, "file", 4), 0);
    expect("middle_self_twice", lget(D "/self/self/ls/g", "user.a"), 7);
    expect("final_after_middle", lget(D "/ls/lf", "user.a"), -ENODATA);
    expect("final_after_self", lget(D "/self/lf", "user.a"), -ENODATA);
    expect("final_chain_link", lget(D "/ls2", "user.a"), -ENODATA);
    expect("final_self_link", lget(D "/self", "user.a"), -ENODATA);
    expect("dirlink_slash", lget(D "/ls/", "user.a"), 6);
    check("dirlink_slash_value", mem_eq(buf, "subdir", 6), 0);
    expect("dirlink_dot", lget(D "/ls/.", "user.a"), 6);
    expect("chain_slash", lget(D "/ls2/", "user.a"), 6);
    expect("self_slash", lget(D "/self/", "user.a"), 3);
    check("self_slash_value", mem_eq(buf, "top", 3), 0);
    expect("dirlink_dotdot", lget(D "/ls/..", "user.a"), 3);
    expect("filelink_slash_enotdir", lget(D "/lf/", "user.a"), -ENOTDIR);
    expect("filelink_dot_enotdir", lget(D "/lf/.", "user.a"), -ENOTDIR);
    expect("filelink_dotdot_enotdir", lget(D "/lf/..", "user.a"), -ENOTDIR);
    expect("filelink_middle_enotdir", lget(D "/lf/x", "user.a"), -ENOTDIR);
    expect("dangling_slash_enoent", lget(D "/dangling/", "user.a"), -ENOENT);
    expect("dangling_middle_enoent", lget(D "/dangling/x", "user.a"), -ENOENT);
    expect("loop_slash_eloop", lget(D "/loop/", "user.a"), -ELOOP);
    expect("loop_middle_eloop", lget(D "/loop/x", "user.a"), -ELOOP);
    expect("chdir", syscall1(SYS_chdir, D), 0);
    expect("relative_middle", lget("ls/g", "user.a"), 7);
    expect("relative_final_link", lget("lf", "user.a"), -ENODATA);
    expect("relative_dot", lget(".", "user.a"), 3);
    expect("chdir_back", syscall1(SYS_chdir, "/"), 0);
    expect("cleanup_sub_lf", a_unlink(D "/sub/lf"), 0);
    expect("cleanup_g", a_unlink(D "/sub/g"), 0);
    expect("cleanup_sub", a_rmdir(D "/sub"), 0);
    expect("cleanup_ls", a_unlink(D "/ls"), 0);
    expect("cleanup_ls2", a_unlink(D "/ls2"), 0);
    expect("cleanup_labs", a_unlink(D "/labs"), 0);
    expect("cleanup_self", a_unlink(D "/self"), 0);
    expect("cleanup_lf", a_unlink(D "/lf"), 0);
    expect("cleanup_dangling", a_unlink(D "/dangling"), 0);
    expect("cleanup_loop", a_unlink(D "/loop"), 0);
    expect("cleanup_f", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
