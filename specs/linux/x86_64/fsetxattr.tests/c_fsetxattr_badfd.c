/* fsetxattr descriptor errors: EBADF for -1, a number never opened, a
 * closed descriptor, and an O_PATH descriptor (of a file or of a
 * symbolic link opened with O_NOFOLLOW). The argument checks come
 * first: bad flags (EINVAL), the name (EFAULT, ERANGE), the size
 * (E2BIG) and reading the value (EFAULT) are reported even with a bad
 * descriptor; the namespace (EOPNOTSUPP) only for a good one.
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

#define F "/tmp/c_fsetxattr_badfd_f"
#define L "/tmp/c_fsetxattr_badfd_l"

static char big[XATTR_SIZE_MAX + 1];

int test_main(void) {
    a_unlink(F);
    a_unlink(L);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_link", a_symlink(F, L), 0);
    i64 pfd = a_open(F, O_PATH, 0);
    i64 lfd = a_open(L, O_PATH | O_NOFOLLOW, 0);
    i64 cfd = a_open(F, O_RDWR, 0);
    check("setup_fds", pfd >= 0 && lfd >= 0 && cfd >= 0, pfd);
    expect("close", a_close(cfd), 0);

    expect("minus1_ebadf", x_fset(-1, "user.a", "x", 1, 0), -EBADF);
    expect("unused_ebadf", x_fset(900, "user.a", "x", 1, 0), -EBADF);
    expect("closed_ebadf", x_fset(cfd, "user.a", "x", 1, 0), -EBADF);
    expect("opath_ebadf", x_fset(pfd, "user.a", "x", 1, 0), -EBADF);
    expect("opath_link_ebadf", x_fset(lfd, "trusted.a", "x", 1, 0), -EBADF);
    expect("big_fd_ebadf", x_fset(0x7fffffff, "user.a", "x", 1, 0), -EBADF);

    expect("flags_before_fd", x_fset(-1, "user.a", "x", 1, 8), -EINVAL);
    expect("empty_name_before_fd", x_fset(-1, "", "x", 1, 0), -ERANGE);
    expect("null_name_before_fd", x_fset(-1, 0, "x", 1, 0), -EFAULT);
    expect("size_before_fd", x_fset(-1, "user.a", big, XATTR_SIZE_MAX + 1, 0), -E2BIG);
    expect("value_before_fd", x_fset(-1, "user.a", 0, 1, 0), -EFAULT);
    expect("fd_before_namespace", x_fset(-1, "plain", "x", 1, 0), -EBADF);
    expect("fd_before_prefix_only", x_fset(pfd, "user.", "x", 1, 0), -EBADF);
    expect("nothing_set", x_list(F, 0, 0), 0);

    a_close(pfd);
    a_close(lfd);
    a_unlink(L);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
