/* swapoff opens its target for writing: a regular file on a read-only
 * mount is EROFS (30), even for root, also through a symlink; a
 * directory there is still EISDIR (21) and a missing name ENOENT (2).
 * Once the mount is read-write again the same file opens and is EINVAL
 * (22, not a swap area). Runs on a tmpfs in a mount namespace of its own.
 * poly-diff: needs=root (only root may mount and get past CAP_SYS_ADMIN)
 */
#include "poly_ipc.h"

#define EISDIR 21

#define SYS_swapoff 168

#define D "/tmp/swapoff_root_rofs"
#define F D "/file"

static void make_file(const char *path, const void *data, i64 len, int mode) {
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    i64 fd = syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY | O_TRUNC, mode);
    if (len > 0) syscall3(SYS_write, fd, data, len);
    syscall1(SYS_close, fd);
}

#define SYS_unshare 272
#define CLONE_NEWNS 0x00020000

/* Run body in a child with a mount namespace of its own (every mount
 * private) and a fresh tmpfs on D. The child never calls body unless
 * all of that worked. Returns the child's exit code. */
static int in_private_tmpfs(void (*body)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_NEWNS);
        check("unshare_newns", r == 0, r);
        if (r != 0) exit_group(1);
        r = syscall5(SYS_mount, "none", "/", 0, MS_REC | MS_PRIVATE, 0);
        check("make_private", r == 0, r);
        if (r != 0) exit_group(1);
        r = syscall5(SYS_mount, "x", D, "tmpfs", 0, "size=1m");
        check("mount_tmpfs", r == 0, r);
        if (r != 0) exit_group(1);
        body();
        syscall2(SYS_umount2, D, MNT_DETACH);
        exit_group(failed ? 1 : 0);
    }
    int code = pid > 0 ? wait_exit(pid) : -1;
    check("fork", pid > 0, pid);
    return code;
}

static char zeros[8192];

static void body(void) {
    make_file(F, zeros, sizeof zeros, 0600);
    syscall3(SYS_mkdirat, AT_FDCWD, D "/sub", 0755);
    syscall3(SYS_symlinkat, "file", AT_FDCWD, D "/link");
    i64 r = syscall5(SYS_mount, "x", D, 0, MS_REMOUNT | MS_RDONLY, 0);
    check("remount_ro", r == 0, r);
    if (r != 0) return;
    r = syscall1(SYS_swapoff, F);
    check("readonly_erofs", r == -EROFS, r);
    r = syscall1(SYS_swapoff, D "/link");
    check("readonly_symlink_erofs", r == -EROFS, r);
    r = syscall1(SYS_swapoff, D "/sub");
    check("readonly_dir_eisdir", r == -EISDIR, r);
    r = syscall1(SYS_swapoff, D "/missing");
    check("readonly_missing_enoent", r == -ENOENT, r);
    r = syscall5(SYS_mount, "x", D, 0, MS_REMOUNT, 0);
    check("remount_rw", r == 0, r);
    r = syscall1(SYS_swapoff, F);
    check("readwrite_einval", r == -EINVAL, r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, D, 0755);
    int code = in_private_tmpfs(body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, D, AT_REMOVEDIR) == 0, 0);
    return finish();
}
