/* On anything but a symbolic link lsetxattr is setxattr: create and
 * replace with flags 0, XATTR_CREATE (EEXIST), XATTR_REPLACE (ENODATA),
 * other flag bits EINVAL, empty values, name rules (ERANGE, EOPNOTSUPP,
 * EINVAL for "user."), E2BIG above 65536 bytes, EFAULT for an
 * unreadable value; on a directory it works, on a FIFO user.* is EPERM.
 * The check order is the same: flags, name, size, value, then the path.
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

#define F "/tmp/c_lsetxattr_regular_f"
#define D "/tmp/c_lsetxattr_regular_d"
#define FIFO "/tmp/c_lsetxattr_regular_fifo"
#define MISSING "/tmp/c_lsetxattr_regular_missing"

static char buf[64];
static char big[XATTR_SIZE_MAX + 1];

int test_main(void) {
    a_unlink(F);
    a_unlink(FIFO);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_dir", a_mkdir(D, 0755), 0);
    expect("setup_fifo", syscall4(SYS_mknodat, AT_FDCWD, FIFO, S_IFIFO | 0644, 0), 0);

    expect("create", x_lset(F, "user.a", "one", 3, 0), 0);
    expect("get", x_get(F, "user.a", buf, sizeof buf), 3);
    expect("replace", x_lset(F, "user.a", "four", 4, 0), 0);
    expect("get_replaced", x_lget(F, "user.a", buf, sizeof buf), 4);
    check("replaced_value", mem_eq(buf, "four", 4), buf[0]);
    expect("create_existing_eexist", x_lset(F, "user.a", "x", 1, XATTR_CREATE), -EEXIST);
    expect("replace_missing_enodata", x_lset(F, "user.b", "x", 1, XATTR_REPLACE), -ENODATA);
    expect("both_flags_missing_enodata", x_lset(F, "user.b", "x", 1, 3), -ENODATA);
    expect("other_flag_einval", x_lset(F, "user.a", "x", 1, 4), -EINVAL);
    expect("empty_value", x_lset(F, "user.e", 0, 0, 0), 0);
    expect("empty_value_len", x_get(F, "user.e", buf, sizeof buf), 0);
    expect("empty_name_erange", x_lset(F, "", "x", 1, 0), -ERANGE);
    expect("unknown_prefix_eopnotsupp", x_lset(F, "plain", "x", 1, 0), -EOPNOTSUPP);
    expect("prefix_only_einval", x_lset(F, "user.", "x", 1, 0), -EINVAL);
    expect("too_big_e2big", x_lset(F, "user.big", big, XATTR_SIZE_MAX + 1, 0), -E2BIG);
    expect("null_value_efault", x_lset(F, "user.n", 0, 1, 0), -EFAULT);
    expect("dir_ok", x_lset(D, "user.d", "d", 1, 0), 0);
    expect("dir_get", x_get(D, "user.d", buf, sizeof buf), 1);
    expect("fifo_eperm", x_lset(FIFO, "user.f", "f", 1, 0), -EPERM);

    expect("order_flags_first", x_lset(MISSING, "", big, XATTR_SIZE_MAX + 1, 4), -EINVAL);
    expect("order_name_second", x_lset(MISSING, "", big, XATTR_SIZE_MAX + 1, 0), -ERANGE);
    expect("order_size_third", x_lset(MISSING, "user.a", big, XATTR_SIZE_MAX + 1, 0), -E2BIG);
    expect("order_value_fourth", x_lset(MISSING, "user.a", 0, 1, 0), -EFAULT);
    expect("order_path_fifth", x_lset(MISSING, "plain", "x", 1, 0), -ENOENT);
    expect("null_path_efault", x_lset(0, "user.a", "x", 1, 0), -EFAULT);
    expect("empty_path_enoent", x_lset("", "user.a", "x", 1, 0), -ENOENT);

    a_unlink(FIFO);
    a_rmdir(D);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
