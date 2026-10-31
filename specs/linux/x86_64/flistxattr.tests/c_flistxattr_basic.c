/* flistxattr(fd, list, size) is listxattr on an open descriptor's file:
 * NUL-terminated names, 0 for none, size 0 for the length, an exact
 * buffer works, one byte short is ERANGE, a larger buffer gets only the
 * names, sizes above 65536 are fine, a NULL buffer with enough size is
 * EFAULT. Any access mode works (O_WRONLY too), as does a directory
 * descriptor, and an unlinked but open file still lists its names.
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

#define F "/tmp/c_flistxattr_basic_f"
#define D "/tmp/c_flistxattr_basic_d"

static char buf[128];

int test_main(void) {
    a_unlink(F);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_dir", a_mkdir(D, 0755), 0);
    i64 ro = a_open(F, O_RDONLY, 0);
    i64 wo = a_open(F, O_WRONLY, 0);
    i64 dfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    check("setup_fds", ro >= 0 && wo >= 0 && dfd >= 0, ro);

    expect("none", x_flist(ro, buf, sizeof buf), 0);
    expect("none_size0", x_flist(ro, 0, 0), 0);
    expect("set_x", x_fset(ro, "user.x", "1", 1, 0), 0);
    expect("set_yy", x_fset(ro, "user.yy", "", 0, 0), 0);
    expect("size0", x_flist(ro, 0, 0), 15);
    memset(buf, '#', sizeof buf);
    i64 n = x_flist(ro, buf, 15);
    expect("exact", n, 15);
    check("has_x", x_has(buf, n, "user.x"), 0);
    check("has_yy", x_has(buf, n, "user.yy"), 0);
    check("rest_untouched", buf[15] == '#', buf[15]);
    expect("short_erange", x_flist(ro, buf, 14), -ERANGE);
    expect("larger", x_flist(ro, buf, sizeof buf), 15);
    expect("huge", x_flist(ro, buf, 1UL << 36), 15);
    expect("null_buffer_efault", x_flist(ro, 0, 64), -EFAULT);
    expect("null_buffer_short_erange", x_flist(ro, 0, 4), -ERANGE);
    expect("wronly_fd", x_flist(wo, buf, sizeof buf), 15);
    expect("dir_none", x_flist(dfd, buf, sizeof buf), 0);
    expect("dir_set", x_fset(dfd, "user.d", "d", 1, 0), 0);
    expect("dir_one", x_flist(dfd, buf, sizeof buf), 7);
    expect("unlink", a_unlink(F), 0);
    expect("unlinked_lists", x_flist(wo, buf, sizeof buf), 15);
    expect("unlinked_remove", x_fremove(ro, "user.x"), 0);
    expect("unlinked_after_remove", x_flist(ro, buf, sizeof buf), 8);

    a_close(ro);
    a_close(wo);
    a_close(dfd);
    expect("cleanup", a_rmdir(D), 0);
    return finish();
}
