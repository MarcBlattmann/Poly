/* A successful setxattr or removexattr changes the file's ctime (to the
 * current time) and nothing else: mtime, size and contents stay. Reading
 * and listing change no time. A failed call (ENODATA, EEXIST) leaves
 * ctime alone. Each step waits 30 ms first, so a change is visible with
 * the coarse clock file systems use for timestamps.
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

#define F "/tmp/c_removexattr_ctime_f"

static char buf[64];

static void pause_30ms(void) {
    struct a_timespec ts = {0, 30000000};
    syscall2(SYS_nanosleep, &ts, 0);
}
static int same_time(u64 s1, u64 n1, u64 s2, u64 n2) { return s1 == s2 && n1 == n2; }

int test_main(void) {
    a_unlink(F);
    expect("setup_file", a_make_file(F, "contents", 0644), 0);
    struct a_stat a, b;
    expect("stat_0", a_stat_path(F, &a), 0);

    pause_30ms();
    expect("set", x_set(F, "user.t", "x", 1, 0), 0);
    expect("stat_1", a_stat_path(F, &b), 0);
    check("set_changes_ctime", !same_time(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);
    check("set_keeps_mtime", same_time(a.st_mtime_sec, a.st_mtime_nsec, b.st_mtime_sec, b.st_mtime_nsec), 0);
    expect("set_keeps_size", b.st_size, 8);

    pause_30ms();
    expect("get", x_get(F, "user.t", buf, sizeof buf), 1);
    expect("list", x_list(F, buf, sizeof buf), 7);
    expect("stat_2", a_stat_path(F, &a), 0);
    check("get_list_keep_ctime", same_time(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);

    pause_30ms();
    expect("failed_remove", x_remove(F, "user.none"), -ENODATA);
    expect("failed_create", x_set(F, "user.t", "y", 1, XATTR_CREATE), -EEXIST);
    expect("failed_replace", x_set(F, "user.none", "y", 1, XATTR_REPLACE), -ENODATA);
    expect("stat_3", a_stat_path(F, &b), 0);
    check("failures_keep_ctime", same_time(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);

    pause_30ms();
    expect("replace", x_set(F, "user.t", "z", 1, 0), 0);
    expect("stat_4", a_stat_path(F, &a), 0);
    check("replace_changes_ctime", !same_time(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);

    pause_30ms();
    expect("remove", x_remove(F, "user.t"), 0);
    expect("stat_5", a_stat_path(F, &b), 0);
    check("remove_changes_ctime", !same_time(a.st_ctime_sec, a.st_ctime_nsec, b.st_ctime_sec, b.st_ctime_nsec), 0);
    check("remove_keeps_mtime", same_time(a.st_mtime_sec, a.st_mtime_nsec, b.st_mtime_sec, b.st_mtime_nsec), 0);
    check("ctime_not_before_mtime", b.st_ctime_sec > b.st_mtime_sec ||
                                        (b.st_ctime_sec == b.st_mtime_sec && b.st_ctime_nsec >= b.st_mtime_nsec), 0);

    i64 fd = a_open(F, O_RDONLY, 0);
    expect("read_contents", a_read(fd, buf, sizeof buf), 8);
    check("contents_unchanged", mem_eq(buf, "contents", 8), buf[0]);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
