/* file_getattr(2) errors: usize below 24 EINVAL, above a page E2BIG;
 * at_flags other than AT_SYMLINK_NOFOLLOW|AT_EMPTY_PATH EINVAL; an
 * unwritable buffer EFAULT; a file whose file system has no inode flags
 * (a symlink itself, a device node such as /dev/null) EOPNOTSUPP; a
 * missing file ENOENT; "" without AT_EMPTY_PATH ENOENT; a closed dirfd
 * EBADF.
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

#define F "/dev/shm/e_file_getattr_errors"
#define L "/dev/shm/e_file_getattr_errors.link"

int test_main(void) {
    make_file(F, 0644);
    unlink_path(L);
    syscall3(SYS_symlinkat, F, AT_FDCWD, L);
    struct k_file_attr fa;
    i64 r;
    r = file_getattr(AT_FDCWD, F, &fa, 16, 0);
    check("usize_16_einval", r == -EINVAL, r);
    r = file_getattr(AT_FDCWD, F, &fa, 0, 0);
    check("usize_0_einval", r == -EINVAL, r);
    r = file_getattr(AT_FDCWD, F, &fa, 4097, 0);
    check("usize_4097_e2big", r == -E2BIG, r);
    r = file_getattr(AT_FDCWD, F, &fa, sizeof fa, 0x400);
    check("at_flags_einval", r == -EINVAL, r);
    r = file_getattr(AT_FDCWD, F, (struct k_file_attr *)8, sizeof fa, 0);
    check("buffer_efault", r == -EFAULT, r);
    r = file_getattr(AT_FDCWD, L, &fa, sizeof fa, AT_SYMLINK_NOFOLLOW);
    check("symlink_itself_eopnotsupp", r == -EOPNOTSUPP, r);
    r = file_getattr(AT_FDCWD, L, &fa, sizeof fa, 0);
    check("symlink_followed_ok", r == 0, r);
    r = file_getattr(AT_FDCWD, "/dev/null", &fa, sizeof fa, 0);
    check("device_eopnotsupp", r == -EOPNOTSUPP, r);
    r = file_getattr(AT_FDCWD, "/dev/shm/e_file_getattr_errors.none", &fa, sizeof fa, 0);
    check("missing_enoent", r == -ENOENT, r);
    i64 fd = open3(F, O_RDONLY, 0);
    r = file_getattr((int)fd, "", &fa, sizeof fa, 0);
    check("empty_without_flag_enoent", r == -ENOENT, r);
    r = file_getattr(99, "x", &fa, sizeof fa, 0);
    check("closed_dirfd_ebadf", r == -EBADF, r);
    close((int)fd);
    unlink_path(L);
    unlink_path(F);
    return finish();
}
