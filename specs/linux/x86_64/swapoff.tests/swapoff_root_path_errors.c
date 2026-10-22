/* With CAP_SYS_ADMIN, swapoff resolves the path like open: a missing
 * file ENOENT (2), an empty string ENOENT, a relative missing name
 * (resolved from the cwd) ENOENT, a symlink to a missing file ENOENT, a
 * path through a regular file ENOTDIR (20), a trailing slash on a
 * regular file ENOTDIR, a symlink loop ELOOP (40), a name of 4096 bytes
 * ENAMETOOLONG (36), NULL and a bad pointer EFAULT (14). Nothing is
 * created.
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define EISDIR 21

#define SYS_swapoff 168
#define SYS_newfstatat 262

#define F "/tmp/swapoff_root_path_file"
#define L "/tmp/swapoff_root_path_loop"
#define S "/tmp/swapoff_root_path_dangling"

static char longname[4200];
static char st[256];

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, F, 0);
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, F, O_CREAT | O_WRONLY | O_TRUNC, 0600));
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_symlinkat, L, AT_FDCWD, L);
    syscall3(SYS_unlinkat, AT_FDCWD, S, 0);
    syscall3(SYS_symlinkat, "/tmp/swapoff_root_path_missing", AT_FDCWD, S);
    longname[0] = '/';
    for (int i = 1; i < 4096; i++) longname[i] = 'a';
    longname[4096] = 0;

    i64 r = syscall1(SYS_swapoff, "/tmp/swapoff_root_path_missing");
    check("missing_enoent", r == -ENOENT, r);
    r = syscall1(SYS_swapoff, "");
    check("empty_enoent", r == -ENOENT, r);
    r = syscall1(SYS_swapoff, "tmp/swapoff_root_path_missing");
    check("relative_missing_enoent", r == -ENOENT, r);
    r = syscall1(SYS_swapoff, S);
    check("dangling_symlink_enoent", r == -ENOENT, r);
    r = syscall1(SYS_swapoff, F "/x");
    check("through_file_enotdir", r == -ENOTDIR, r);
    r = syscall1(SYS_swapoff, F "/");
    check("trailing_slash_enotdir", r == -ENOTDIR, r);
    r = syscall1(SYS_swapoff, L);
    check("loop_eloop", r == -ELOOP, r);
    r = syscall1(SYS_swapoff, longname);
    check("too_long_enametoolong", r == -ENAMETOOLONG, r);
    r = syscall1(SYS_swapoff, 0);
    check("null_efault", r == -EFAULT, r);
    r = syscall1(SYS_swapoff, 8);
    check("bad_pointer_efault", r == -EFAULT, r);
    r = syscall4(SYS_newfstatat, AT_FDCWD, "/tmp/swapoff_root_path_missing", st, 0);
    check("missing_not_created", r == -ENOENT, r);

    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_unlinkat, AT_FDCWD, S, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, F, 0) == 0, 0);
    return finish();
}
