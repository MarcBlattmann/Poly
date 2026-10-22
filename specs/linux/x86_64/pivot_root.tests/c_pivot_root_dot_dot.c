/* pivot_root(".", ".") with the current directory at a mount point
 * succeeds (new_root and put_old may be the same directory); a
 * following umount2(".", MNT_DETACH) removes the old root, after which
 * "/" is the new root (its file is there, the old root's /tmp is not)
 * and getcwd is "/".
 * poly-diff: needs=root (only root may pivot_root)
 */
#include "poly_ipc.h"

#define SYS_pivot_root 155
#define SYS_getcwd 79
#define SYS_chdir 80
#define SYS_unshare 272
#define SYS_newfstatat 262
#define CLONE_NEWNS 0x00020000
#define MS_SHARED (1 << 20) /* linux/mount.h */

struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 unused[3];
};

static __attribute__((unused)) i64 stat_path(const char *path, struct k_stat *st) {
    return syscall4(SYS_newfstatat, AT_FDCWD, path, st, 0);
}

static __attribute__((unused)) void make_file(const char *path) {
    syscall1(SYS_close, syscall4(SYS_openat, AT_FDCWD, path, O_CREAT | O_WRONLY, 0644));
}

static __attribute__((unused)) int cwd_is(const char *want) {
    char buf[256];
    i64 r = syscall2(SYS_getcwd, buf, sizeof buf);
    return r == (i64)str_len(want) + 1 && str_eq(buf, want);
}

/* Run `body` in a child that has a mount namespace of its own in which
 * every mount is private, so nothing it does reaches the caller's mounts.
 * The child never calls body when that setup fails. Returns the child's
 * exit code. */
static int in_private_ns(void (*body)(void)) {
    i64 pid = fork();
    if (pid == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_NEWNS);
        check("unshare_newns", r == 0, r);
        if (r != 0) exit_group(1);
        r = syscall5(SYS_mount, "none", "/", 0, MS_REC | MS_PRIVATE, 0);
        check("make_private", r == 0, r);
        if (r != 0) exit_group(1);
        body();
        exit_group(failed ? 1 : 0);
    }
    return wait_exit(pid);
}
#define N "/tmp/c_pivot_root_dot"

static void body(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    make_file(N "/marker");
    syscall1(SYS_chdir, N);
    r = syscall2(SYS_pivot_root, ".", ".");
    check("pivot_dot_dot_ok", r == 0, r);
    if (r != 0) return;
    r = syscall2(SYS_umount2, ".", MNT_DETACH);
    check("detach_old_root", r == 0, r);
    struct k_stat st;
    r = stat_path("/marker", &st);
    check("new_root_visible", r == 0, r);
    r = stat_path("/tmp", &st);
    check("old_root_gone", r == -ENOENT, r);
    check("cwd_is_root", cwd_is("/"), 0);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, N, 0755);
    int code = in_private_ns(body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR) == 0, 0);
    return finish();
}
