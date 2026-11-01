/* Sizes on tmpfs (the file system at /dev/shm, and the kind of file
 * system an initramfs is): a user.* value of the full 65536 bytes
 * (XATTR_SIZE_MAX) is accepted and read back whole, and one file takes
 * many such values (here 20, 1.25 MiB); hundreds of attributes per file
 * are fine. (Disk file systems have smaller limits: ext4 without large
 * attribute support says ENOSPC above about one block in total.)
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

/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

#define F "/dev/shm/c_setxattr_tmpfs_f"

static char val[XATTR_SIZE_MAX + 16];
static char got[XATTR_SIZE_MAX + 16];

static const char *nth(char *out, const char *prefix, int i) {
    char *p = str_cpy(out, prefix);
    p[0] = (char)('0' + i / 100);
    p[1] = (char)('0' + i / 10 % 10);
    p[2] = (char)('0' + i % 10);
    p[3] = 0;
    return out;
}

int test_main(void) {
    struct a_statfs sf;
    i64 r = syscall2(SYS_statfs, "/dev/shm", &sf);
    check("setup_dev_shm_is_tmpfs", r == 0 && sf.f_type == TMPFS_MAGIC, r == 0 ? (i64)sf.f_type : r);
    a_unlink(F);
    expect("setup_file", a_make_file(F, "", 0644), 0);

    for (int i = 0; i < XATTR_SIZE_MAX; i++) val[i] = (char)(i * 7 + i / 256);
    expect("set_65536", x_set(F, "user.max", val, XATTR_SIZE_MAX, 0), 0);
    expect("get_65536_size0", x_get(F, "user.max", 0, 0), XATTR_SIZE_MAX);
    expect("get_65536", x_get(F, "user.max", got, XATTR_SIZE_MAX), XATTR_SIZE_MAX);
    check("value_65536_bytes", mem_eq(got, val, XATTR_SIZE_MAX), got[XATTR_SIZE_MAX - 1]);
    expect("get_65536_bigger_buf", x_get(F, "user.max", got, sizeof got), XATTR_SIZE_MAX);
    expect("get_65535_buf_erange", x_get(F, "user.max", got, XATTR_SIZE_MAX - 1), -ERANGE);
    expect("set_65537_e2big", x_set(F, "user.max", val, XATTR_SIZE_MAX + 1, 0), -E2BIG);

    char name[32];
    int ok = 1;
    for (int i = 0; i < 20; i++) ok &= x_set(F, nth(name, "user.big", i), val, XATTR_SIZE_MAX, 0) == 0;
    check("twenty_full_values", ok, 0);
    expect("get_big019", x_get(F, "user.big019", got, sizeof got), XATTR_SIZE_MAX);
    check("big019_bytes", mem_eq(got, val, XATTR_SIZE_MAX), got[0]);
    ok = 1;
    for (int i = 0; i < 300; i++) ok &= x_set(F, nth(name, "user.small", i), "", 0, 0) == 0;
    check("three_hundred_small", ok, 0);
    i64 n = x_list(F, got, sizeof got);
    expect("list_len", n, 9 + 20 * 12 + 300 * 14);
    expect("list_count", x_count(got, n), 321);
    ok = 1;
    for (int i = 0; i < 20; i++) ok &= x_remove(F, nth(name, "user.big", i)) == 0;
    check("remove_big", ok, 0);
    expect("list_len_after", x_list(F, 0, 0), 9 + 300 * 14);

    expect("cleanup", a_unlink(F), 0);
    return finish();
}
