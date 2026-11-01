/* On anything but a symbolic link lgetxattr is getxattr: size 0 gives
 * the length, an exact or larger buffer the value, a smaller one
 * ERANGE, sizes above 65536 are fine, a missing name is ENODATA, a
 * NULL buffer with enough size EFAULT, and name errors (ERANGE, EFAULT)
 * come before path errors (ENOENT, ENOTDIR, EFAULT), which come before
 * namespace errors (EOPNOTSUPP, EINVAL for "user.").
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

#define F "/tmp/c_lgetxattr_regular_f"
#define D "/tmp/c_lgetxattr_regular_d"
#define MISSING "/tmp/c_lgetxattr_regular_missing"

static char buf[64];

int test_main(void) {
    a_unlink(F);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_attr", x_set(F, "user.v", "12345", 5, 0), 0);
    expect("setup_dir", a_mkdir(D, 0755), 0);
    expect("setup_dir_attr", x_set(D, "user.v", "dd", 2, 0), 0);

    expect("size0", x_lget(F, "user.v", 0, 0), 5);
    expect("exact", x_lget(F, "user.v", buf, 5), 5);
    check("value", mem_eq(buf, "12345", 5), buf[0]);
    expect("short_erange", x_lget(F, "user.v", buf, 4), -ERANGE);
    expect("huge_size", x_lget(F, "user.v", buf, 1UL << 40), 5);
    expect("missing_enodata", x_lget(F, "user.w", buf, sizeof buf), -ENODATA);
    expect("null_buffer_efault", x_lget(F, "user.v", 0, 16), -EFAULT);
    expect("dir", x_lget(D, "user.v", buf, sizeof buf), 2);

    expect("empty_name_erange", x_lget(F, "", buf, sizeof buf), -ERANGE);
    expect("null_name_efault", x_lget(F, 0, buf, sizeof buf), -EFAULT);
    expect("name_before_path", x_lget(MISSING, "", buf, sizeof buf), -ERANGE);
    expect("missing_enoent", x_lget(MISSING, "user.v", buf, sizeof buf), -ENOENT);
    expect("path_before_namespace", x_lget(MISSING, "plain", buf, sizeof buf), -ENOENT);
    expect("trailing_slash_enotdir", x_lget(F "/", "user.v", buf, sizeof buf), -ENOTDIR);
    expect("null_path_efault", x_lget(0, "user.v", buf, sizeof buf), -EFAULT);
    expect("empty_path_enoent", x_lget("", "user.v", buf, sizeof buf), -ENOENT);
    expect("unknown_prefix_eopnotsupp", x_lget(F, "plain", buf, sizeof buf), -EOPNOTSUPP);
    expect("prefix_only_einval", x_lget(F, "user.", buf, sizeof buf), -EINVAL);

    a_rmdir(D);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
