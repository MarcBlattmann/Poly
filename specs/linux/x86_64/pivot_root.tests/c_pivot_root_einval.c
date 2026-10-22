/* pivot_root EINVAL cases, as root in a private mount namespace (all
 * mounts private): new_root a directory that is not a mount point
 * (inside a tmpfs mount T); put_old not at or below new_root (another
 * tmpfs mount U); put_old reached with ".." out of new_root. After each
 * failure the root is unchanged; the right arguments then succeed.
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
#define T "/tmp/c_pivot_root_einval_t"
#define U "/tmp/c_pivot_root_einval_u"

static void body(void) {
    i64 r = syscall5(SYS_mount, "x", T, "tmpfs", 0, 0);
    check("mount_t", r == 0, r);
    r |= syscall5(SYS_mount, "x", U, "tmpfs", 0, 0);
    check("mount_u", r == 0, r);
    if (r != 0) return;
    syscall3(SYS_mkdirat, AT_FDCWD, T "/sub", 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, T "/sub/old", 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, T "/old", 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, U "/old", 0755);
    make_file(T "/marker");
    syscall1(SYS_chdir, "/");
    struct k_stat before, after;
    stat_path("/", &before);

    r = syscall2(SYS_pivot_root, T "/sub", T "/sub/old");
    check("not_mount_point_einval", r == -EINVAL, r);
    r = syscall2(SYS_pivot_root, T, U);
    check("put_old_other_mount_einval", r == -EINVAL, r);
    r = syscall2(SYS_pivot_root, T, U "/old");
    check("put_old_below_other_einval", r == -EINVAL, r);
    r = syscall2(SYS_pivot_root, T, T "/../c_pivot_root_einval_u/old");
    check("put_old_dotdot_out_einval", r == -EINVAL, r);
    stat_path("/", &after);
    check("root_unchanged", after.dev == before.dev && after.ino == before.ino, 0);
    check("cwd_unchanged", cwd_is("/"), 0);

    r = syscall2(SYS_pivot_root, T, T "/old");
    check("then_pivot_ok", r == 0, r);
    r = stat_path("/marker", &after);
    check("new_root_visible", r == 0, r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, T, 0755);
    syscall3(SYS_mkdirat, AT_FDCWD, U, 0755);
    int code = in_private_ns(body);
    check("child", code == 0, code);
    syscall3(SYS_unlinkat, AT_FDCWD, U, AT_REMOVEDIR);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, T, AT_REMOVEDIR) == 0, 0);
    return finish();
}
