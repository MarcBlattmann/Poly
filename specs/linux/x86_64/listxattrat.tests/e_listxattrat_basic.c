/* listxattrat(2) stores the names of the attributes of (dfd, path) the
 * caller may see, each NUL-terminated, and returns the total length; size
 * 0 (list may be NULL) returns the length needed. A buffer too small is
 * ERANGE. AT_SYMLINK_NOFOLLOW lists a symlink's own (none here);
 * AT_EMPTY_PATH lists dfd. Errors: bad at_flags EINVAL; an unwritable list
 * EFAULT; a missing file ENOENT. A file with no attributes gives 0.
 * The order of names is not specified (the test does not rely on it).
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

#define F "/tmp/e_listxattrat_basic"
#define L "/tmp/e_listxattrat_basic.link"

int test_main(void) {
    make_file(F, 0644);
    unlink_path(L);
    syscall3(SYS_symlinkat, F, AT_FDCWD, L);
    char list[256];
    i64 r = listxattrat(AT_FDCWD, F, 0, list, sizeof list);
    check("none_yet", r == 0, r);
    xset(F, "user.alpha", "1", 0);
    xset(F, "user.bb", "22", 0);
    xset(F, "user.c", "333", 0);
    memset(list, 0, sizeof list);
    r = listxattrat(AT_FDCWD, F, 0, list, sizeof list);
    check("total_length", r == 11 + 8 + 7, r);
    check("has_alpha", list_has(list, r, "user.alpha"), 0);
    check("has_bb", list_has(list, r, "user.bb"), 0);
    check("has_c", list_has(list, r, "user.c"), 0);
    r = listxattrat(AT_FDCWD, F, 0, 0, 0);
    check("size_query", r == 26, r);
    r = listxattrat(AT_FDCWD, F, 0, list, 25);
    check("small_erange", r == -ERANGE, r);
    r = listxattrat(AT_FDCWD, F, 0, list, 26);
    check("exact_size", r == 26, r);
    r = listxattrat(AT_FDCWD, L, 0, list, sizeof list);
    check("follows_link", r == 26, r);
    r = listxattrat(AT_FDCWD, L, AT_SYMLINK_NOFOLLOW, list, sizeof list);
    check("link_itself_empty", r == 0, r);
    i64 fd = open3(F, O_RDONLY, 0);
    r = listxattrat((int)fd, "", AT_EMPTY_PATH, list, sizeof list);
    check("empty_path", r == 26, r);
    r = listxattrat((int)fd, "", 0, list, sizeof list);
    check("empty_path_without_flag_enoent", r == -ENOENT, r);
    r = listxattrat(AT_FDCWD, F, 0x400, list, sizeof list);
    check("at_flags_einval", r == -EINVAL, r);
    r = listxattrat(AT_FDCWD, F, 0, (char *)8, sizeof list);
    check("list_efault", r == -EFAULT, r);
    r = listxattrat(AT_FDCWD, "/tmp/e_listxattrat_basic.none", 0, list, sizeof list);
    check("missing_enoent", r == -ENOENT, r);
    r = listxattrat(AT_FDCWD, F, 0, list, 1 << 20);
    check("huge_size_ok", r == 26, r);
    close((int)fd);
    unlink_path(L);
    unlink_path(F);
    return finish();
}
