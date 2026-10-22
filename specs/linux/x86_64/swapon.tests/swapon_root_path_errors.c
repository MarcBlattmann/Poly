/* With CAP_SYS_ADMIN and valid flags, swapon resolves the path like
 * open: a missing file ENOENT (2), an empty string ENOENT, a relative
 * missing name (resolved from the cwd) ENOENT, a symlink to a missing
 * file ENOENT, a path through a regular file ENOTDIR (20), a trailing
 * slash on a regular file ENOTDIR, a symlink loop ELOOP (40), a name of
 * 4096 bytes ENAMETOOLONG (36), a bad pointer EFAULT (14). Nothing is
 * created.
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define SYS_swapon 167
#define SYS_newfstatat 262
#define SYS_swapoff 168

/* A failed swapon must not have enabled anything; if it did, undo it. */
static __attribute__((unused)) void undo(const char *path, i64 r) {
    if (r == 0) syscall1(SYS_swapoff, path);
}

static __attribute__((unused)) void make_file(const char *path, const char *data, i64 len) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, 0600);
    if (len > 0) syscall3(SYS_write, fd, data, len);
    syscall1(SYS_close, fd);
}

#define F "/tmp/swapon_root_path_file"
#define L "/tmp/swapon_root_path_loop"
#define S "/tmp/swapon_root_path_dangling"

static char longname[4200];
static char st[256];

int test_main(void) {
    make_file(F, "x", 1);
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_symlinkat, L, AT_FDCWD, L);
    syscall3(SYS_unlinkat, AT_FDCWD, S, 0);
    syscall3(SYS_symlinkat, "/tmp/swapon_root_path_missing", AT_FDCWD, S);
    longname[0] = '/';
    for (int i = 1; i < 4096; i++) longname[i] = 'a';
    longname[4096] = 0;

    i64 r = syscall2(SYS_swapon, "/tmp/swapon_root_path_missing", 0);
    check("missing_enoent", r == -ENOENT, r);
    r = syscall2(SYS_swapon, "", 0);
    check("empty_enoent", r == -ENOENT, r);
    r = syscall2(SYS_swapon, "tmp/swapon_root_path_missing", 0x8000 | 1);
    check("relative_missing_enoent", r == -ENOENT, r);
    r = syscall2(SYS_swapon, S, 0);
    check("dangling_symlink_enoent", r == -ENOENT, r);
    r = syscall2(SYS_swapon, F "/x", 0);
    check("through_file_enotdir", r == -ENOTDIR, r);
    undo(F, r);
    r = syscall2(SYS_swapon, F "/", 0);
    check("trailing_slash_enotdir", r == -ENOTDIR, r);
    undo(F, r);
    r = syscall2(SYS_swapon, L, 0);
    check("loop_eloop", r == -ELOOP, r);
    r = syscall2(SYS_swapon, longname, 0);
    check("too_long_enametoolong", r == -ENAMETOOLONG, r);
    r = syscall2(SYS_swapon, 8, 0);
    check("bad_pointer_efault", r == -EFAULT, r);
    r = syscall2(SYS_swapon, 0, 0);
    check("null_efault", r == -EFAULT, r);
    r = syscall4(SYS_newfstatat, AT_FDCWD, "/tmp/swapon_root_path_missing", st, 0);
    check("missing_not_created", r == -ENOENT, r);

    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, S, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
