/* pivot_root moves the caller's current directory only when it is the
 * old root: a current directory elsewhere on the old root stays and is
 * now seen below put_old (getcwd "/old/tmp/..."), one inside new_root
 * stays too and is seen from the new root ("/sub"). Files opened before
 * the call keep working. Each case in its own private mount namespace.
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
#define N "/tmp/c_pivot_root_cwd"
#define OTHER "/tmp/c_pivot_root_cwd_other"

static void elsewhere(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    syscall3(SYS_mkdirat, AT_FDCWD, N "/old", 0755);
    make_file(N "/marker");
    i64 fd = syscall4(SYS_openat, AT_FDCWD, OTHER "/file", O_CREAT | O_WRONLY, 0644);
    syscall1(SYS_chdir, OTHER);
    r = syscall2(SYS_pivot_root, N, N "/old");
    check("pivot_ok", r == 0, r);
    if (r != 0) return;
    check("cwd_below_put_old", cwd_is("/old" OTHER), 0);
    struct k_stat st;
    r = stat_path("file", &st);
    check("cwd_still_works", r == 0, r);
    r = stat_path("/marker", &st);
    check("root_changed", r == 0, r);
    r = syscall3(SYS_write, fd, "abc", 3);
    check("open_file_works", r == 3, r);
}

static void inside(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    syscall3(SYS_mkdirat, AT_FDCWD, N "/old", 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, N "/sub", 0755);
    syscall1(SYS_chdir, N "/sub");
    r = syscall2(SYS_pivot_root, N, N "/old");
    check("pivot_ok", r == 0, r);
    if (r != 0) return;
    check("cwd_seen_from_new_root", cwd_is("/sub"), 0);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, N, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, OTHER, 0755);
    int code = in_private_ns(elsewhere);
    check("child_elsewhere", code == 0, code);
    code = in_private_ns(inside);
    check("child_inside", code == 0, code);
    syscall3(SYS_unlinkat, AT_FDCWD, OTHER "/file", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, OTHER, AT_REMOVEDIR);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR) == 0, 0);
    return finish();
}
