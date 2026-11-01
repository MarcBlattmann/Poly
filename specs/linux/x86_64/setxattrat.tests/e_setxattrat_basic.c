/* setxattrat(2) sets an extended attribute on (dfd, path), like setxattr
 * but with a dirfd, at_flags and the value passed in struct xattr_args
 * {u64 value, u32 size, u32 flags}. flags is XATTR_CREATE (fail with
 * EEXIST if present) or XATTR_REPLACE (fail with ENODATA if absent) or 0.
 * at_flags: AT_SYMLINK_NOFOLLOW (act on a symlink itself), AT_EMPTY_PATH
 * (path "" or NULL means dfd). A zero-length value is allowed. The value
 * is read back with getxattrat or getxattr.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_setxattrat 463
#define SYS_getxattrat 464
#define SYS_listxattrat 465
#define SYS_removexattrat 466
#define SYS_file_getattr 468
#define SYS_file_setattr 469
#define SYS_getxattr 191
#define SYS_fchmod 91
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EBADF 9
#define EFAULT 14
#define EINVAL 22
#define E2BIG 7
#define ERANGE 34
#define ENODATA 61
/* linux/fcntl.h */
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_NO_AUTOMOUNT 0x800
#define AT_EMPTY_PATH 0x1000
/* linux/xattr.h */
#define XATTR_CREATE 1
#define XATTR_REPLACE 2
/* struct xattr_args (linux/xattr.h), XATTR_ARGS_SIZE_VER0 16. */
struct k_xattr_args {
    u64 value;
    unsigned size;
    unsigned flags;
};
/* struct file_attr (linux/fs.h), FILE_ATTR_SIZE_VER0 24. */
struct k_file_attr {
    u64 fa_xflags;
    unsigned fa_extsize, fa_nextents, fa_projid, fa_cowextsize;
};
/* FS_XFLAG_* (linux/fs.h) */
#define FS_XFLAG_IMMUTABLE 0x00000008
#define FS_XFLAG_APPEND 0x00000010
#define FS_XFLAG_NOATIME 0x00000040
#define FS_XFLAG_NODUMP 0x00000080

static __attribute__((unused)) i64 setxattrat(int dfd, const char *path, unsigned at, const char *name,
                                              struct k_xattr_args *a, u64 usize) {
    return syscall6(SYS_setxattrat, dfd, (i64)path, at, (i64)name, (i64)a, (i64)usize);
}
static __attribute__((unused)) i64 getxattrat(int dfd, const char *path, unsigned at, const char *name,
                                              struct k_xattr_args *a, u64 usize) {
    return syscall6(SYS_getxattrat, dfd, (i64)path, at, (i64)name, (i64)a, (i64)usize);
}
static __attribute__((unused)) i64 listxattrat(int dfd, const char *path, unsigned at, char *list, u64 size) {
    return syscall5(SYS_listxattrat, dfd, path, at, list, size);
}
static __attribute__((unused)) i64 removexattrat(int dfd, const char *path, unsigned at, const char *name) {
    return syscall4(SYS_removexattrat, dfd, path, at, name);
}
static __attribute__((unused)) i64 file_getattr(int dfd, const char *path, struct k_file_attr *fa, u64 usize,
                                                unsigned at) {
    return syscall5(SYS_file_getattr, dfd, path, fa, usize, at);
}
static __attribute__((unused)) i64 file_setattr(int dfd, const char *path, struct k_file_attr *fa, u64 usize,
                                                unsigned at) {
    return syscall5(SYS_file_setattr, dfd, path, fa, usize, at);
}
/* Set name to the C string val on path (follows links). */
static __attribute__((unused)) i64 xset(const char *path, const char *name, const char *val, unsigned flags) {
    struct k_xattr_args a = { (u64)val, (unsigned)str_len(val), flags };
    return setxattrat(AT_FDCWD, path, 0, name, &a, sizeof a);
}
static __attribute__((unused)) i64 xget(const char *path, const char *name, char *buf, unsigned size) {
    struct k_xattr_args a = { (u64)buf, size, 0 };
    return getxattrat(AT_FDCWD, path, 0, name, &a, sizeof a);
}
static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
static __attribute__((unused)) void make_file(const char *path, i64 mode) {
    unlink_path(path);
    i64 fd = open3(path, O_CREAT | O_RDWR | O_EXCL, mode);
    if (fd >= 0) close((int)fd);
}
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}
static __attribute__((unused)) int drop_to_user(void) {
    if (syscall3(SYS_setresgid, 1000, 1000, 1000) != 0) return 0;
    if (syscall3(SYS_setresuid, 1000, 1000, 1000) != 0) return 0;
    return 1;
}
/* Does the NUL-separated list (len bytes) contain name? */
static __attribute__((unused)) int list_has(const char *list, i64 len, const char *name) {
    i64 i = 0;
    while (i < len) {
        if (str_eq(list + i, name)) return 1;
        i += (i64)str_len(list + i) + 1;
    }
    return 0;
}

#define F "/tmp/e_setxattrat_basic"
#define L "/tmp/e_setxattrat_basic.link"

int test_main(void) {
    make_file(F, 0644);
    unlink_path(L);
    syscall3(SYS_symlinkat, F, AT_FDCWD, L);
    struct k_xattr_args a = { (u64) "one", 3, 0 };
    i64 r = setxattrat(AT_FDCWD, F, 0, "user.k", &a, sizeof a);
    check("set", r == 0, r);
    char buf[32];
    memset(buf, 0, sizeof buf);
    r = syscall4(SYS_getxattr, F, "user.k", buf, sizeof buf);
    check("seen_by_getxattr", r == 3 && mem_eq(buf, "one", 3), r);

    a.flags = XATTR_CREATE;
    r = setxattrat(AT_FDCWD, F, 0, "user.k", &a, sizeof a);
    check("create_existing_eexist", r == -EEXIST, r);
    a.flags = XATTR_REPLACE;
    r = setxattrat(AT_FDCWD, F, 0, "user.none", &a, sizeof a);
    check("replace_missing_enodata", r == -ENODATA, r);
    a.value = (u64) "two!";
    a.size = 4;
    r = setxattrat(AT_FDCWD, F, 0, "user.k", &a, sizeof a);
    check("replace_existing", r == 0 && xget(F, "user.k", buf, sizeof buf) == 4 && mem_eq(buf, "two!", 4), r);
    a.flags = XATTR_CREATE;
    r = setxattrat(AT_FDCWD, F, 0, "user.new", &a, sizeof a);
    check("create_new", r == 0, r);

    struct k_xattr_args e = { 0, 0, 0 };
    r = setxattrat(AT_FDCWD, F, 0, "user.empty", &e, sizeof e);
    check("empty_value", r == 0 && xget(F, "user.empty", buf, sizeof buf) == 0, r);

    a.flags = 0;
    r = setxattrat(AT_FDCWD, L, 0, "user.via_link", &a, sizeof a);
    check("follows_symlink", r == 0 && xget(F, "user.via_link", buf, sizeof buf) == 4, r);
    r = setxattrat(AT_FDCWD, L, AT_SYMLINK_NOFOLLOW, "user.on_link", &a, sizeof a);
    check("user_ns_on_symlink_eperm", r == -EPERM, r);

    i64 fd = open3(F, O_RDONLY, 0);
    r = setxattrat((int)fd, "", AT_EMPTY_PATH, "user.byfd", &a, sizeof a);
    check("empty_path_fd", r == 0 && xget(F, "user.byfd", buf, sizeof buf) == 4, r);
    r = setxattrat((int)fd, 0, AT_EMPTY_PATH, "user.nullpath", &a, sizeof a);
    check("null_path_with_empty_path", r == 0, r);
    i64 dfd = open3("/tmp", O_RDONLY | O_DIRECTORY, 0);
    r = setxattrat((int)dfd, "e_setxattrat_basic", 0, "user.rel", &a, sizeof a);
    check("relative_to_dirfd", r == 0 && xget(F, "user.rel", buf, sizeof buf) == 4, r);
    close((int)fd);
    close((int)dfd);
    unlink_path(L);
    unlink_path(F);
    return finish();
}
