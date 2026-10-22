/* swapoff opens its target for writing, so for a privileged caller a
 * directory is EISDIR (21), not EINVAL: "/", "/tmp", "/tmp/" with a
 * trailing slash, a fresh directory of mode 0000, and a symlink to a
 * directory.
 * poly-diff: needs=root (only root gets past the CAP_SYS_ADMIN check)
 */
#include "poly_ipc.h"

#define EISDIR 21

#define SYS_swapoff 168
#define SYS_newfstatat 262

#define D "/tmp/swapoff_root_dir_d"
#define L "/tmp/swapoff_root_dir_link"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR);
    i64 r = syscall3(SYS_mkdirat, AT_FDCWD, D, 0);
    check("mkdir", r == 0, r);
    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    syscall3(SYS_symlinkat, "/tmp", AT_FDCWD, L);

    r = syscall1(SYS_swapoff, "/");
    check("root_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, "/tmp");
    check("tmp_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, "/tmp/");
    check("tmp_slash_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, D);
    check("mode0_dir_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, L);
    check("symlink_dir_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, "tmp");
    check("relative_dir_eisdir", r == -EISDIR, r);

    syscall3(SYS_unlinkat, AT_FDCWD, L, 0);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
