/* setxattr names. A name is a NUL-terminated string of 1..255 bytes
 * (XATTR_NAME_MAX, prefix included): empty or longer is ERANGE. It must
 * start with a known namespace prefix, matched case-sensitively with its
 * dot: "user.", "trusted.", "security." or "system."; anything else
 * ("plain", "user" without the dot, "USER.x", "other.x") is EOPNOTSUPP,
 * and so is a "system." name the file system does not know. A prefix
 * with nothing after it ("user.") is EINVAL. After the prefix any bytes
 * go: dots, slashes, a repeated prefix, bytes >= 0x80. Each such name is
 * its own attribute. An unreadable name is EFAULT, also a name that runs
 * into an unmapped page before its NUL; a name of 256 or more readable
 * bytes is ERANGE even if its NUL is never reached.
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

#define F "/tmp/c_setxattr_names_f"

static char buf[512];
static char name[300];

/* "user." followed by n-5 copies of 'n': a name of n bytes. */
static const char *long_name(int n) {
    str_cpy(name, "user.");
    for (int i = 5; i < n; i++) name[i] = 'n';
    name[n] = 0;
    return name;
}

int test_main(void) {
    a_unlink(F);
    expect("setup_file", a_make_file(F, "", 0644), 0);

    expect("empty_erange", x_set(F, "", "x", 1, 0), -ERANGE);
    expect("len_255_ok", x_set(F, long_name(255), "x", 1, 0), 0);
    expect("len_255_get", x_get(F, long_name(255), buf, sizeof buf), 1);
    expect("len_256_erange", x_set(F, long_name(256), "x", 1, 0), -ERANGE);
    expect("len_299_erange", x_set(F, long_name(299), "x", 1, 0), -ERANGE);

    expect("no_namespace_eopnotsupp", x_set(F, "plain", "x", 1, 0), -EOPNOTSUPP);
    expect("user_without_dot_eopnotsupp", x_set(F, "user", "x", 1, 0), -EOPNOTSUPP);
    expect("upper_case_prefix_eopnotsupp", x_set(F, "USER.x", "x", 1, 0), -EOPNOTSUPP);
    expect("other_prefix_eopnotsupp", x_set(F, "other.x", "x", 1, 0), -EOPNOTSUPP);
    expect("unknown_system_eopnotsupp", x_set(F, "system.nothing", "x", 1, 0), -EOPNOTSUPP);
    expect("leading_space_eopnotsupp", x_set(F, " user.x", "x", 1, 0), -EOPNOTSUPP);
    expect("prefix_only_einval", x_set(F, "user.", "x", 1, 0), -EINVAL);

    expect("dots_ok", x_set(F, "user.a.b.c", "1", 1, 0), 0);
    expect("slash_ok", x_set(F, "user.a/b", "2", 1, 0), 0);
    expect("repeated_prefix_ok", x_set(F, "user.user.", "3", 1, 0), 0);
    expect("high_byte_ok", x_set(F, "user.\xc3\xa9t\xff", "4", 1, 0), 0);
    expect("get_dots", x_get(F, "user.a.b.c", buf, sizeof buf), 1);
    check("dots_value", buf[0] == '1', buf[0]);
    expect("get_slash", x_get(F, "user.a/b", buf, sizeof buf), 1);
    check("slash_value", buf[0] == '2', buf[0]);
    expect("get_repeated", x_get(F, "user.user.", buf, sizeof buf), 1);
    check("repeated_value", buf[0] == '3', buf[0]);
    expect("get_high", x_get(F, "user.\xc3\xa9t\xff", buf, sizeof buf), 1);
    check("high_value", buf[0] == '4', buf[0]);
    i64 n = x_list(F, buf, sizeof buf);
    expect("list_len", n, 255 + 1 + 11 + 9 + 11 + 10);
    expect("list_count", x_count(buf, n), 5);
    check("list_has_slash", x_has(buf, n, "user.a/b"), 0);
    check("list_has_high", x_has(buf, n, "user.\xc3\xa9t\xff"), 0);

    expect("null_name_efault", x_set(F, 0, "x", 1, 0), -EFAULT);
    expect("low_name_efault", x_set(F, (void *)16, "x", 1, 0), -EFAULT);
    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("setup_map", (i64)pg > 0, (i64)pg);
    expect("setup_unmap", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    char *end = pg + A_PAGE;
    memcpy(end - 6, "user.x", 6);
    expect("name_into_unmapped_efault", x_set(F, end - 6, "x", 1, 0), -EFAULT);
    memcpy(end - 256, "user.", 5);
    for (int i = 5; i < 256; i++) end[-256 + i] = 'q';
    expect("name_256_unterminated_erange", x_set(F, end - 256, "x", 1, 0), -ERANGE);
    memcpy(end - 255, "user.", 5);
    expect("name_255_unterminated_efault", x_set(F, end - 255, "x", 1, 0), -EFAULT);

    expect("cleanup", a_unlink(F), 0);
    return finish();
}
