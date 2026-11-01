/* setxattr(path, name, value, size, flags) with flags 0 creates a user.*
 * attribute or replaces its value and returns 0. Values are bytes, not
 * strings: embedded and trailing NUL bytes are kept, and a value may be
 * empty (size 0, value pointer NULL or not). Replacing with a shorter or
 * longer value takes the new length. Names are case-sensitive. The
 * value is copied at the call: changing the caller's buffer afterwards
 * changes nothing. A path naming a symbolic link sets the attribute on
 * the file the link points to. Directories take user.* attributes too.
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

#define F "/tmp/c_setxattr_basic_f"
#define L "/tmp/c_setxattr_basic_l"
#define D "/tmp/c_setxattr_basic_d"

static char buf[256];

int test_main(void) {
    a_unlink(F);
    a_unlink(L);
    a_rmdir(D);
    expect("setup_file", a_make_file(F, "data", 0644), 0);

    expect("set_create", x_set(F, "user.colour", "blue", 4, 0), 0);
    expect("get_len", x_get(F, "user.colour", buf, sizeof buf), 4);
    check("get_value", mem_eq(buf, "blue", 4), buf[0]);

    expect("set_replace_longer", x_set(F, "user.colour", "turquoise", 9, 0), 0);
    expect("get_longer", x_get(F, "user.colour", buf, sizeof buf), 9);
    check("longer_value", mem_eq(buf, "turquoise", 9), buf[0]);
    expect("set_replace_shorter", x_set(F, "user.colour", "red", 3, 0), 0);
    memset(buf, '#', 16);
    expect("get_shorter", x_get(F, "user.colour", buf, sizeof buf), 3);
    check("shorter_value_and_rest_untouched", mem_eq(buf, "red###", 6), buf[3]);

    static const char bin[6] = {0, 'a', 0, 0, 'b', 0};
    expect("set_binary", x_set(F, "user.bin", bin, 6, 0), 0);
    expect("get_binary_len", x_get(F, "user.bin", buf, sizeof buf), 6);
    check("binary_value", mem_eq(buf, bin, 6), buf[1]);

    expect("set_empty", x_set(F, "user.empty", "x", 0, 0), 0);
    expect("get_empty", x_get(F, "user.empty", buf, sizeof buf), 0);
    expect("set_empty_null", x_set(F, "user.empty2", 0, 0, 0), 0);
    expect("get_empty_null", x_get(F, "user.empty2", buf, sizeof buf), 0);
    expect("replace_with_empty", x_set(F, "user.colour", 0, 0, 0), 0);
    expect("get_replaced_empty", x_get(F, "user.colour", buf, sizeof buf), 0);

    expect("set_lower", x_set(F, "user.case", "lower", 5, 0), 0);
    expect("set_upper", x_set(F, "user.CASE", "UPPER", 5, 0), 0);
    expect("get_lower", x_get(F, "user.case", buf, sizeof buf), 5);
    check("lower_value", mem_eq(buf, "lower", 5), buf[0]);
    expect("get_upper", x_get(F, "user.CASE", buf, sizeof buf), 5);
    check("upper_value", mem_eq(buf, "UPPER", 5), buf[0]);

    char val[4] = {'o', 'n', 'e', '1'};
    expect("set_copy", x_set(F, "user.copy", val, 4, 0), 0);
    val[0] = 'X';
    expect("get_copy", x_get(F, "user.copy", buf, sizeof buf), 4);
    check("value_copied_at_call", mem_eq(buf, "one1", 4), buf[0]);

    expect("setup_symlink", a_symlink(F, L), 0);
    expect("set_through_link", x_set(L, "user.vialink", "v", 1, 0), 0);
    expect("on_target", x_get(F, "user.vialink", buf, sizeof buf), 1);
    expect("not_on_link", x_lget(L, "user.vialink", buf, sizeof buf), -ENODATA);

    expect("setup_dir", a_mkdir(D, 0755), 0);
    expect("set_on_dir", x_set(D, "user.dir", "d", 1, 0), 0);
    expect("get_on_dir", x_get(D, "user.dir", buf, sizeof buf), 1);

    struct a_stat st;
    expect("stat", a_stat_path(F, &st), 0);
    expect("file_size_unchanged", st.st_size, 4);

    expect("cleanup_link", a_unlink(L), 0);
    expect("cleanup_file", a_unlink(F), 0);
    expect("cleanup_dir", a_rmdir(D), 0);
    return finish();
}
