/* The privilege pivot_root checks is CAP_SYS_ADMIN in the effective
 * set: root that removes only CAP_SYS_ADMIN (21) from its effective set
 * (keeping it permitted, and keeping all other capabilities) gets EPERM
 * for a call that would work; after putting it back into the effective
 * set the same call succeeds. In a private mount namespace.
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
#define N "/tmp/c_pivot_root_cap"

struct cap_header {
    unsigned version;
    int pid;
};
struct cap_data {
    unsigned effective, permitted, inheritable;
};

static void body(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    syscall3(SYS_mkdirat, AT_FDCWD, N "/old", 0755);
    syscall1(SYS_chdir, "/");

    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    syscall2(SYS_capget, &h, d);
    d[0].effective &= ~(1U << 21); /* CAP_SYS_ADMIN, linux/capability.h */
    r = syscall2(SYS_capset, &h, d);
    check("drop_sys_admin_effective", r == 0, r);
    r = syscall2(SYS_pivot_root, N, N "/old");
    check("without_sys_admin_eperm", r == -EPERM, r);
    d[0].effective |= 1U << 21;
    r = syscall2(SYS_capset, &h, d);
    check("restore_sys_admin", r == 0, r);
    r = syscall2(SYS_pivot_root, N, N "/old");
    check("with_sys_admin_ok", r == 0, r);
}

int test_main(void) {
    syscall3(SYS_mkdirat, AT_FDCWD, N, 0755);
    int code = in_private_ns(body);
    check("child", code == 0, code);
    check("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR) == 0, 0);
    return finish();
}
