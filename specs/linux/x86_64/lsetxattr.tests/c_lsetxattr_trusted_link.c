/* trusted.* and security.* attributes on a symbolic link itself (on
 * tmpfs, /dev/shm): root can set them with lsetxattr, also on a
 * dangling link; lgetxattr and llistxattr see them, while getxattr and
 * listxattr through the link see the target's attributes instead.
 * XATTR_CREATE/XATTR_REPLACE work as on files. Without CAP_SYS_ADMIN
 * (uid 1000), even on its own link: trusted.* is EPERM to set and
 * ENODATA to read, security.* EPERM to set but readable.
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

#define F "/dev/shm/c_lsetxattr_trusted_link_f"
#define L "/dev/shm/c_lsetxattr_trusted_link_l"
#define DL "/dev/shm/c_lsetxattr_trusted_link_dangling"
#define UL "/dev/shm/c_lsetxattr_trusted_link_userlink"

static char buf[128];

static int as_user(void) {
    expect("drop_to_1000", x_become(1000, 1000), 0);
    expect("setup_user_link", a_symlink(F, UL), 0);
    expect("user_trusted_own_link_eperm", x_lset(UL, "trusted.t", "x", 1, 0), -EPERM);
    expect("user_security_own_link_eperm", x_lset(UL, "security.s", "x", 1, 0), -EPERM);
    expect("user_user_own_link_eperm", x_lset(UL, "user.u", "x", 1, 0), -EPERM);
    expect("user_read_trusted_enodata", x_lget(L, "trusted.t", buf, sizeof buf), -ENODATA);
    expect("user_read_security", x_lget(L, "security.s", buf, sizeof buf), 2);
    expect("user_llist", x_llist(L, buf, sizeof buf), 11);
    expect("user_cleanup_link", a_unlink(UL), 0);
    return 0;
}

int test_main(void) {
    a_unlink(F);
    a_unlink(L);
    a_unlink(DL);
    a_unlink(UL);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    expect("setup_target_attr", x_set(F, "user.target", "tgt", 3, 0), 0);
    expect("setup_link", a_symlink(F, L), 0);
    expect("setup_dangling", a_symlink("/dev/shm/c_lsetxattr_trusted_link_nowhere", DL), 0);

    expect("lset_trusted", x_lset(L, "trusted.t", "lt", 2, 0), 0);
    expect("lset_security", x_lset(L, "security.s", "ls", 2, 0), 0);
    expect("lget_trusted", x_lget(L, "trusted.t", buf, sizeof buf), 2);
    check("lget_trusted_value", mem_eq(buf, "lt", 2), buf[0]);
    expect("get_through_link_target", x_get(L, "trusted.t", buf, sizeof buf), -ENODATA);
    i64 n = x_llist(L, buf, sizeof buf);
    expect("llist_link", n, 21);
    check("llist_has_trusted", x_has(buf, n, "trusted.t"), 0);
    check("llist_has_security", x_has(buf, n, "security.s"), 0);
    n = x_list(L, buf, sizeof buf);
    expect("list_through_link", n, 12);
    check("list_target_name", x_has(buf, n, "user.target"), 0);
    expect("target_has_no_trusted", x_get(F, "trusted.t", buf, sizeof buf), -ENODATA);
    expect("create_existing_eexist", x_lset(L, "trusted.t", "x", 1, XATTR_CREATE), -EEXIST);
    expect("replace_missing_enodata", x_lset(L, "trusted.none", "x", 1, XATTR_REPLACE), -ENODATA);
    expect("replace_existing", x_lset(L, "trusted.t", "new", 3, XATTR_REPLACE), 0);
    expect("replaced_len", x_lget(L, "trusted.t", buf, sizeof buf), 3);
    expect("prefix_only_einval", x_lset(L, "trusted.", "x", 1, 0), -EINVAL);
    expect("dangling_lset_trusted", x_lset(DL, "trusted.d", "d", 1, 0), 0);
    expect("dangling_lget_trusted", x_lget(DL, "trusted.d", buf, sizeof buf), 1);
    expect("dangling_llist", x_llist(DL, buf, sizeof buf), 10);

    check("user_checks", x_in_child(as_user), 0);

    a_unlink(DL);
    a_unlink(L);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
