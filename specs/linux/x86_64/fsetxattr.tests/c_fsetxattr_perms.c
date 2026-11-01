/* fsetxattr checks permission against the file's current mode and the
 * caller's current identity, not against how the descriptor was
 * opened. As uid 1000: an O_RDWR descriptor of its own file, opened
 * while the mode was 0600, gives EACCES after chmod 0400 and works
 * again after chmod 0600; an O_RDONLY descriptor of root's 0666 file
 * works; a descriptor of root's 0644 file opened by root before the
 * switch to uid 1000 gives EACCES. trusted.* and security.* are EPERM.
 * poly-diff: needs=root (sets up files of other users and drops to uid 1000)
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

#define OWN "/tmp/c_fsetxattr_perms_own"
#define OPENF "/tmp/c_fsetxattr_perms_open"
#define ROOTF "/tmp/c_fsetxattr_perms_root"

static i64 root_fd;

static int as_user(void) {
    expect("drop_to_1000", x_become(1000, 1000), 0);
    i64 fd = a_open(OWN, O_RDWR, 0);
    check("open_own_rdwr", fd >= 0, fd);
    expect("own_0600_ok", x_fset(fd, "user.a", "1", 1, 0), 0);
    expect("chmod_0400", a_chmod(OWN, 0400), 0);
    expect("own_0400_eacces", x_fset(fd, "user.b", "2", 1, 0), -EACCES);
    expect("own_0400_remove_eacces", x_fremove(fd, "user.a"), -EACCES);
    expect("chmod_0600", a_chmod(OWN, 0600), 0);
    expect("own_0600_again_ok", x_fset(fd, "user.b", "2", 1, 0), 0);
    expect("own_trusted_eperm", x_fset(fd, "trusted.t", "t", 1, 0), -EPERM);
    expect("own_security_eperm", x_fset(fd, "security.s", "s", 1, 0), -EPERM);
    i64 ofd = a_open(OPENF, O_RDONLY, 0);
    check("open_0666_rdonly", ofd >= 0, ofd);
    expect("other_0666_rdonly_fd_ok", x_fset(ofd, "user.o", "o", 1, 0), 0);
    expect("root_opened_fd_eacces", x_fset(root_fd, "user.r", "r", 1, 0), -EACCES);
    return 0;
}

int test_main(void) {
    a_unlink(OWN);
    a_unlink(OPENF);
    a_unlink(ROOTF);
    expect("setup_own", a_make_file(OWN, "", 0600), 0);
    expect("setup_own_chown", a_chown(OWN, 1000, 1000), 0);
    expect("setup_open", a_make_file(OPENF, "", 0644), 0);
    expect("setup_open_mode", a_chmod(OPENF, 0666), 0);
    expect("setup_root", a_make_file(ROOTF, "", 0644), 0);
    root_fd = a_open(ROOTF, O_RDWR, 0);
    check("setup_root_fd", root_fd >= 0, root_fd);

    check("user_checks", x_in_child(as_user), 0);

    expect("root_fd_as_root_ok", x_fset(root_fd, "user.r", "r", 1, 0), 0);
    a_close(root_fd);
    a_unlink(OWN);
    a_unlink(OPENF);
    expect("cleanup", a_unlink(ROOTF), 0);
    return finish();
}
