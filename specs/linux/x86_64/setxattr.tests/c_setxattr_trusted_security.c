/* The trusted.* and security.* namespaces. Root (CAP_SYS_ADMIN) can set
 * trusted.* and security.* attributes on regular files, directories,
 * FIFOs and symbolic links themselves (no LSM claims the names used
 * here). "trusted." and "security." alone are EINVAL. Without
 * CAP_SYS_ADMIN (uid 1000): setting or removing trusted.* is EPERM
 * (also "trusted." alone), checked before file permissions; reading
 * one is ENODATA and listxattr leaves trusted.* names out. security.*
 * names can be read and are listed, but setting or removing one is
 * EPERM, even on the caller's own writable file. On tmpfs (/dev/shm,
 * used here) listxattr's size query without CAP_SYS_ADMIN counts only
 * the names it would list; ext4 counts the hidden trusted.* names too
 * (and then wants a buffer that large), so this test stays on tmpfs.
 * poly-diff: needs=root (only root has CAP_SYS_ADMIN; drops to uid 1000)
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

#define F "/dev/shm/c_setxattr_trusted_security_f"
#define G "/dev/shm/c_setxattr_trusted_security_g"
#define FIFO "/dev/shm/c_setxattr_trusted_security_fifo"

static char buf[128];

static int as_user(void) {
    expect("drop_to_1000", x_become(1000, 1000), 0);
    expect("user_set_trusted_eperm", x_set(F, "trusted.t2", "x", 1, 0), -EPERM);
    expect("user_set_trusted_prefix_only_eperm", x_set(F, "trusted.", "x", 1, 0), -EPERM);
    expect("user_set_trusted_own_file_eperm", x_set(G, "trusted.t2", "x", 1, 0), -EPERM);
    expect("user_get_trusted_enodata", x_get(F, "trusted.t", buf, sizeof buf), -ENODATA);
    expect("user_remove_trusted_eperm", x_remove(F, "trusted.t"), -EPERM);
    expect("user_set_security_eperm", x_set(F, "security.s2", "x", 1, 0), -EPERM);
    expect("user_set_security_own_file_eperm", x_set(G, "security.s2", "x", 1, 0), -EPERM);
    expect("user_get_security", x_get(F, "security.s", buf, sizeof buf), 2);
    check("user_security_value", mem_eq(buf, "ss", 2), buf[0]);
    expect("user_remove_security_eperm", x_remove(F, "security.s"), -EPERM);
    i64 n = x_list(F, buf, sizeof buf);
    expect("user_list_len", n, 18);
    check("user_list_has_user", x_has(buf, n, "user.u"), 0);
    check("user_list_has_security", x_has(buf, n, "security.s"), 0);
    check("user_list_no_trusted", !x_has(buf, n, "trusted.t"), 0);
    expect("user_list_size0", x_list(F, 0, 0), 18);
    expect("user_own_file_user_attr_ok", x_set(G, "user.mine", "m", 1, 0), 0);
    return 0;
}

int test_main(void) {
    a_unlink(F);
    a_unlink(G);
    a_unlink(FIFO);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_user_file", a_make_file(G, "", 0644), 0);
    expect("setup_user_chown", a_chown(G, 1000, 1000), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, FIFO, S_IFIFO | 0644, 0), 0);

    expect("root_set_user", x_set(F, "user.u", "uu", 2, 0), 0);
    expect("root_set_trusted", x_set(F, "trusted.t", "tt", 2, 0), 0);
    expect("root_set_security", x_set(F, "security.s", "ss", 2, 0), 0);
    expect("root_get_trusted", x_get(F, "trusted.t", buf, sizeof buf), 2);
    check("root_trusted_value", mem_eq(buf, "tt", 2), buf[0]);
    expect("root_trusted_prefix_only_einval", x_set(F, "trusted.", "x", 1, 0), -EINVAL);
    expect("root_security_prefix_only_einval", x_set(F, "security.", "x", 1, 0), -EINVAL);
    expect("root_get_trusted_prefix_only_einval", x_get(F, "trusted.", buf, sizeof buf), -EINVAL);
    i64 n = x_list(F, buf, sizeof buf);
    expect("root_list_len", n, 28);
    expect("root_list_count", x_count(buf, n), 3);
    check("root_list_has_trusted", x_has(buf, n, "trusted.t"), 0);
    expect("root_trusted_on_fifo", x_set(FIFO, "trusted.f", "f", 1, 0), 0);
    expect("root_security_on_fifo", x_set(FIFO, "security.f", "f", 1, 0), 0);
    expect("root_user_on_fifo_eperm", x_set(FIFO, "user.f", "f", 1, 0), -EPERM);
    expect("root_fifo_list", x_list(FIFO, buf, sizeof buf), 21);

    check("user_checks", x_in_child(as_user), 0);

    expect("root_remove_trusted", x_remove(F, "trusted.t"), 0);
    expect("root_remove_security", x_remove(F, "security.s"), 0);
    expect("root_list_after", x_list(F, buf, sizeof buf), 7);

    a_unlink(FIFO);
    a_unlink(G);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
