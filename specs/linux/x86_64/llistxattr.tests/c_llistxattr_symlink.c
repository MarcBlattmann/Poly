/* llistxattr lists the attributes of a symbolic link itself: a link
 * without trusted.* or security.* attributes lists nothing (0), whatever
 * its target has; so does a dangling link (listxattr: ENOENT) and a
 * link in a loop (listxattr: ELOOP). On anything else it is listxattr:
 * NUL-terminated names, size 0 for the length, ERANGE for a short
 * buffer, sizes above 65536 fine. Trailing slash and earlier components
 * follow links.
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

#define F "/tmp/c_llistxattr_symlink_f"
#define D "/tmp/c_llistxattr_symlink_d"
#define LF "/tmp/c_llistxattr_symlink_lf"
#define LD "/tmp/c_llistxattr_symlink_ld"
#define DL "/tmp/c_llistxattr_symlink_dangling"
#define L1 "/tmp/c_llistxattr_symlink_l1"
#define L2 "/tmp/c_llistxattr_symlink_l2"

static char buf[128];

int test_main(void) {
    a_unlink(F);
    a_unlink(LF);
    a_unlink(LD);
    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_a", x_set(F, "user.a", "1", 1, 0), 0);
    expect("setup_bb", x_set(F, "user.bb", "2", 1, 0), 0);
    expect("setup_dir", a_mkdir(D, 0755), 0);
    expect("setup_dir_attr", x_set(D, "user.dir", "d", 1, 0), 0);
    expect("setup_lf", a_symlink(F, LF), 0);
    expect("setup_ld", a_symlink(D, LD), 0);
    expect("setup_dangling", a_symlink("/tmp/c_llistxattr_symlink_nowhere", DL), 0);
    expect("setup_loop1", a_symlink(L2, L1), 0);
    expect("setup_loop2", a_symlink(L1, L2), 0);

    expect("link_empty", x_llist(LF, buf, sizeof buf), 0);
    expect("link_size0", x_llist(LF, 0, 0), 0);
    expect("list_follows", x_list(LF, buf, sizeof buf), 15);
    expect("dangling_empty", x_llist(DL, buf, sizeof buf), 0);
    expect("dangling_list_enoent", x_list(DL, buf, sizeof buf), -ENOENT);
    expect("loop_empty", x_llist(L1, buf, sizeof buf), 0);
    expect("loop_list_eloop", x_list(L1, buf, sizeof buf), -ELOOP);

    expect("file_size0", x_llist(F, 0, 0), 15);
    memset(buf, '#', sizeof buf);
    i64 n = x_llist(F, buf, 15);
    expect("file_exact", n, 15);
    check("file_has_a", x_has(buf, n, "user.a"), 0);
    check("file_has_bb", x_has(buf, n, "user.bb"), 0);
    check("file_rest_untouched", buf[15] == '#', buf[15]);
    expect("file_short_erange", x_llist(F, buf, 14), -ERANGE);
    expect("file_huge_size", x_llist(F, buf, 1UL << 33), 15);
    expect("dir", x_llist(D, buf, sizeof buf), 9);
    expect("trailing_slash_dir_link", x_llist(LD "/", buf, sizeof buf), 9);
    expect("trailing_slash_file_link_enotdir", x_llist(LF "/", buf, sizeof buf), -ENOTDIR);
    expect("intermediate_link", x_llist(LD "/../c_llistxattr_symlink_f", buf, sizeof buf), 15);

    expect("null_path_efault", x_llist(0, buf, sizeof buf), -EFAULT);
    expect("missing_enoent", x_llist("/tmp/c_llistxattr_symlink_missing", buf, sizeof buf), -ENOENT);
    expect("empty_path_enoent", x_llist("", buf, sizeof buf), -ENOENT);

    a_unlink(LF);
    a_unlink(LD);
    a_unlink(DL);
    a_unlink(L1);
    a_unlink(L2);
    a_rmdir(D);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
