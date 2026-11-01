/* lsetxattr is setxattr that does not follow a symbolic link in the
 * last component: on a link it acts on the link itself. A link cannot
 * have user.* attributes: EPERM (with any valid flags, also "user."
 * alone, and also for a dangling link or one that is part of a loop,
 * where setxattr says ENOENT or ELOOP). An unknown prefix on a link is
 * EOPNOTSUPP, bad flags EINVAL. The target is not touched; setxattr
 * through the same link sets the target's attribute. With a trailing
 * slash the link is followed (a link to a directory reaches the
 * directory; a link to a file is ENOTDIR), and links in earlier
 * components are always followed.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_socketpair 53
#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_mknodat 259
#define SYS_setxattr 188
#define SYS_lsetxattr 189
#define SYS_fsetxattr 190
#define SYS_getxattr 191
#define SYS_lgetxattr 192
#define SYS_fgetxattr 193
#define SYS_listxattr 194
#define SYS_llistxattr 195
#define SYS_flistxattr 196
#define SYS_removexattr 197
#define SYS_lremovexattr 198
#define SYS_fremovexattr 199
/* asm-generic/errno.h: ENODATA is what <errno.h> also calls ENOATTR */
#define ENODATA 61
/* linux/xattr.h, linux/limits.h */
#define XATTR_CREATE 0x1
#define XATTR_REPLACE 0x2
#define XATTR_NAME_MAX 255
#define XATTR_SIZE_MAX 65536
#define XATTR_LIST_MAX 65536
/* linux/socket.h */
#define AF_UNIX 1
#define SOCK_STREAM 1

static A_UNUSED i64 x_set(const void *path, const void *name, const void *val, u64 size, i64 flags) {
    return syscall5(SYS_setxattr, path, name, val, size, flags);
}
static A_UNUSED i64 x_lset(const void *path, const void *name, const void *val, u64 size, i64 flags) {
    return syscall5(SYS_lsetxattr, path, name, val, size, flags);
}
static A_UNUSED i64 x_fset(i64 fd, const void *name, const void *val, u64 size, i64 flags) {
    return syscall5(SYS_fsetxattr, fd, name, val, size, flags);
}
static A_UNUSED i64 x_get(const void *path, const void *name, void *val, u64 size) {
    return syscall4(SYS_getxattr, path, name, val, size);
}
static A_UNUSED i64 x_lget(const void *path, const void *name, void *val, u64 size) {
    return syscall4(SYS_lgetxattr, path, name, val, size);
}
static A_UNUSED i64 x_fget(i64 fd, const void *name, void *val, u64 size) {
    return syscall4(SYS_fgetxattr, fd, name, val, size);
}
static A_UNUSED i64 x_list(const void *path, void *buf, u64 size) { return syscall3(SYS_listxattr, path, buf, size); }
static A_UNUSED i64 x_llist(const void *path, void *buf, u64 size) { return syscall3(SYS_llistxattr, path, buf, size); }
static A_UNUSED i64 x_flist(i64 fd, void *buf, u64 size) { return syscall3(SYS_flistxattr, fd, buf, size); }
static A_UNUSED i64 x_remove(const void *path, const void *name) { return syscall2(SYS_removexattr, path, name); }
static A_UNUSED i64 x_lremove(const void *path, const void *name) { return syscall2(SYS_lremovexattr, path, name); }
static A_UNUSED i64 x_fremove(i64 fd, const void *name) { return syscall2(SYS_fremovexattr, fd, name); }

static A_UNUSED i64 a_symlink(const char *target, const char *path) {
    return syscall3(SYS_symlinkat, target, AT_FDCWD, path);
}
static A_UNUSED i64 a_chmod(const char *path, i64 mode) { return syscall4(SYS_fchmodat, AT_FDCWD, path, mode, 0); }
static A_UNUSED i64 a_chown(const char *path, i64 uid, i64 gid) {
    return syscall5(SYS_fchownat, AT_FDCWD, path, uid, gid, 0);
}

/* How many names a listxattr result of `len` bytes holds, and whether
 * `name` is one of them (the order of names is not compared: it depends
 * on the file system). */
static A_UNUSED int x_count(const char *list, i64 len) {
    int n = 0;
    for (i64 i = 0; i < len; i++) n += list[i] == 0;
    return n;
}
static A_UNUSED int x_has(const char *list, i64 len, const char *name) {
    for (i64 i = 0; i < len;) {
        if (str_eq(list + i, name)) return 1;
        i += str_len(list + i) + 1;
    }
    return 0;
}

/* Drop to another user (and group, no supplementary groups); only root
 * can. */
static A_UNUSED i64 x_become(i64 uid, i64 gid) {
    i64 r = syscall2(SYS_setgroups, 0, 0);
    if (r == 0) r = syscall3(SYS_setresgid, gid, gid, gid);
    if (r == 0) r = syscall3(SYS_setresuid, uid, uid, uid);
    return r;
}

/* Run `body` in a child with its own copy of memory while this process
 * waits (a vfork-like clone, so the calls of both always come in the same
 * order). The child's checks print as usual; its result is whether all of
 * them passed. */
static A_UNUSED int x_in_child(int (*body)(void)) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        passed = failed = 0;
        int r = body();
        syscall1(SYS_exit_group, r || failed ? 1 : 0);
    }
    i32 status = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return 0;
    return A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0;
}

#define F "/tmp/c_lsetxattr_symlink_f"
#define D "/tmp/c_lsetxattr_symlink_d"
#define LF "/tmp/c_lsetxattr_symlink_lf"
#define LD "/tmp/c_lsetxattr_symlink_ld"
#define DL "/tmp/c_lsetxattr_symlink_dangling"
#define L1 "/tmp/c_lsetxattr_symlink_l1"
#define L2 "/tmp/c_lsetxattr_symlink_l2"

static char buf[64];

int test_main(void) {
    a_unlink(F);
    a_unlink(LF);
    a_unlink(LD);
    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_dir", a_mkdir(D, 0755), 0);
    expect("setup_lf", a_symlink(F, LF), 0);
    expect("setup_ld", a_symlink(D, LD), 0);
    expect("setup_dangling", a_symlink("/tmp/c_lsetxattr_symlink_nowhere", DL), 0);
    expect("setup_loop1", a_symlink(L2, L1), 0);
    expect("setup_loop2", a_symlink(L1, L2), 0);

    expect("link_user_eperm", x_lset(LF, "user.a", "x", 1, 0), -EPERM);
    expect("link_user_create_eperm", x_lset(LF, "user.a", "x", 1, XATTR_CREATE), -EPERM);
    expect("link_user_replace_eperm", x_lset(LF, "user.a", "x", 1, XATTR_REPLACE), -EPERM);
    expect("link_prefix_only_eperm", x_lset(LF, "user.", "x", 1, 0), -EPERM);
    expect("link_unknown_prefix_eopnotsupp", x_lset(LF, "plain", "x", 1, 0), -EOPNOTSUPP);
    expect("link_unknown_system_eopnotsupp", x_lset(LF, "system.nothing", "x", 1, 0), -EOPNOTSUPP);
    expect("link_bad_flags_einval", x_lset(LF, "user.a", "x", 1, 8), -EINVAL);
    expect("link_empty_name_erange", x_lset(LF, "", "x", 1, 0), -ERANGE);
    expect("target_untouched", x_get(F, "user.a", buf, sizeof buf), -ENODATA);
    expect("dangling_lset_eperm", x_lset(DL, "user.a", "x", 1, 0), -EPERM);
    expect("dangling_set_enoent", x_set(DL, "user.a", "x", 1, 0), -ENOENT);
    expect("loop_lset_eperm", x_lset(L1, "user.a", "x", 1, 0), -EPERM);
    expect("loop_set_eloop", x_set(L1, "user.a", "x", 1, 0), -ELOOP);

    expect("set_through_link", x_set(LF, "user.a", "t", 1, 0), 0);
    expect("target_has_it", x_get(F, "user.a", buf, sizeof buf), 1);
    expect("link_has_none", x_lget(LF, "user.a", buf, sizeof buf), -ENODATA);
    expect("link_still_eperm", x_lset(LF, "user.a", "x", 1, 0), -EPERM);

    expect("trailing_slash_dir_link", x_lset(LD "/", "user.d", "d", 1, 0), 0);
    expect("dir_has_it", x_get(D, "user.d", buf, sizeof buf), 1);
    expect("trailing_slash_file_link_enotdir", x_lset(LF "/", "user.a", "x", 1, 0), -ENOTDIR);
    expect("intermediate_link_followed", x_lset(LD "/../c_lsetxattr_symlink_f", "user.i", "i", 1, 0), 0);
    expect("file_has_i", x_get(F, "user.i", buf, sizeof buf), 1);

    a_unlink(LF);
    a_unlink(LD);
    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_rmdir(D);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
