/* removexattr errors and their order: the name first (EFAULT; ERANGE
 * for empty or longer than 255 bytes), before the path; then the path
 * (EFAULT, ENOENT, empty path ENOENT, ENOTDIR, ELOOP, dangling link
 * ENOENT); then for an existing file the namespace (unknown prefix
 * EOPNOTSUPP, "user." alone EINVAL); then ENODATA. On a FIFO a user.*
 * name is EPERM (before ENODATA), as for setxattr.
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

#define F "/tmp/c_removexattr_errors_f"
#define MISSING "/tmp/c_removexattr_errors_missing"
#define DL "/tmp/c_removexattr_errors_dangling"
#define L1 "/tmp/c_removexattr_errors_l1"
#define L2 "/tmp/c_removexattr_errors_l2"
#define FIFO "/tmp/c_removexattr_errors_fifo"

static char lname[300];

int test_main(void) {
    a_unlink(F);
    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_unlink(FIFO);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_attr", x_set(F, "user.a", "a", 1, 0), 0);
    expect("setup_dangling", a_symlink("/tmp/c_removexattr_errors_nowhere", DL), 0);
    expect("setup_loop1", a_symlink(L2, L1), 0);
    expect("setup_loop2", a_symlink(L1, L2), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, FIFO, S_IFIFO | 0644, 0), 0);
    str_cpy(lname, "user.");
    for (int i = 5; i < 256; i++) lname[i] = 'r';

    expect("null_name_efault", x_remove(F, 0), -EFAULT);
    expect("empty_name_erange", x_remove(F, ""), -ERANGE);
    expect("name_256_erange", x_remove(F, lname), -ERANGE);
    expect("name_before_path", x_remove(MISSING, ""), -ERANGE);
    expect("long_name_before_null_path", x_remove(0, lname), -ERANGE);

    expect("null_path_efault", x_remove(0, "user.a"), -EFAULT);
    expect("missing_enoent", x_remove(MISSING, "user.a"), -ENOENT);
    expect("missing_before_namespace", x_remove(MISSING, "plain"), -ENOENT);
    expect("empty_path_enoent", x_remove("", "user.a"), -ENOENT);
    expect("trailing_slash_enotdir", x_remove(F "/", "user.a"), -ENOTDIR);
    expect("loop_eloop", x_remove(L1, "user.a"), -ELOOP);
    expect("dangling_enoent", x_remove(DL, "user.a"), -ENOENT);

    expect("no_namespace_eopnotsupp", x_remove(F, "plain"), -EOPNOTSUPP);
    expect("unknown_system_eopnotsupp", x_remove(F, "system.nothing"), -EOPNOTSUPP);
    expect("prefix_only_einval", x_remove(F, "user."), -EINVAL);
    expect("missing_attr_enodata", x_remove(F, "user.b"), -ENODATA);
    expect("fifo_user_eperm", x_remove(FIFO, "user.a"), -EPERM);
    expect("attr_still_there", x_get(F, "user.a", 0, 0), 1);

    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_unlink(FIFO);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
