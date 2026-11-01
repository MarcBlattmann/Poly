/* listxattr on tmpfs (/dev/shm) with more than 65536 bytes of names
 * (XATTR_LIST_MAX): the size query (size 0) still returns the full
 * length, but any real buffer, even one larger than the list, is E2BIG,
 * since no more than 65536 bytes are ever copied. Below the limit the
 * whole list comes back. Here 6000 names of 12 bytes each (72000
 * bytes), then back to 5000 (60000) after removing 1000.
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

#define F "/dev/shm/c_listxattr_tmpfs_limit_f"

static char buf[100000];

static const char *nth(char *out, int i) {
    char *p = str_cpy(out, "user.s");
    for (int k = 4; k >= 0; k--) {
        p[k] = (char)('0' + i % 10);
        i /= 10;
    }
    p[5] = 0;
    return out;
}

int test_main(void) {
    a_unlink(F);
    expect("setup_file", a_make_file(F, "", 0644), 0);
    char name[32];
    int ok = 1;
    for (int i = 0; i < 6000; i++) ok &= x_set(F, nth(name, i), "", 0, 0) == 0;
    check("set_6000", ok, 0);
    expect("size0_full_length", x_list(F, 0, 0), 72000);
    expect("buffer_65536_e2big", x_list(F, buf, XATTR_LIST_MAX), -E2BIG);
    expect("buffer_100000_e2big", x_list(F, buf, sizeof buf), -E2BIG);
    expect("buffer_small_erange", x_list(F, buf, 100), -ERANGE);
    expect("names_still_readable", x_get(F, "user.s05999", buf, sizeof buf), 0);

    ok = 1;
    for (int i = 5000; i < 6000; i++) ok &= x_remove(F, nth(name, i)) == 0;
    check("remove_1000", ok, 0);
    i64 n = x_list(F, buf, sizeof buf);
    expect("list_60000", n, 60000);
    expect("count_5000", x_count(buf, n), 5000);
    check("has_first", x_has(buf, n, "user.s00000"), 0);
    check("has_last", x_has(buf, n, "user.s04999"), 0);
    check("not_removed", !x_has(buf, n, "user.s05000"), 0);

    expect("cleanup", a_unlink(F), 0);
    return finish();
}
