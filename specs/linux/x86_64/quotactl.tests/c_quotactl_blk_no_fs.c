/* A block device node whose device number no mounted file system uses
 * is ENODEV (19), for every subcommand (Q_SYNC, Q_GETFMT, Q_GETQUOTA,
 * Q_QUOTAON with a bad addr, unknown ones), with or without
 * capabilities (all dropped with capset for the second round). A bad
 * type is still EINVAL first. Made as root on a private tmpfs.
 * poly-diff: needs=root (only root may make block device nodes and mount)
 */
#include "a_sys.h"

/* asm-generic/errno-base.h */
#define ENOTBLK 15

/* asm/unistd_64.h */
#define SYS_capset 126
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_quotactl 179
#define SYS_unshare 272
#define SYS_mknodat 259
#define CLONE_NEWNS 0x00020000

/* linux/mount.h */
#define MS_NODEV 4
#define MS_REC 16384
#define MS_PRIVATE (1 << 18)

/* linux/quota.h */
#define QCMD(cmd, type) ((((cmd) << 8) | ((type) & 0xff)))
#define Q_SYNC 0x800001
#define Q_QUOTAON 0x800002
#define Q_GETFMT 0x800004
#define Q_GETQUOTA 0x800007
#define QFMT_VFS_V1 4

/* mknod's dev argument: major in bits 8-19, minor in bits 0-7 and 20-31. */
#define NO_SUCH_DEV 0xffffffffu /* major 4095, minor 1048575 */
#define NO_SUCH_DEV2 0xfff00u   /* major 4095, minor 0 */

struct cap_header {
    u32 version;
    i32 pid;
};
struct cap_data {
    u32 effective, permitted, inheritable;
};

static i64 drop_caps(void) {
    struct cap_header h = { 0x20080522, 0 }; /* _LINUX_CAPABILITY_VERSION_3 */
    struct cap_data d[2];
    memset(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
}

static i64 qc(i64 cmd, const char *special, i64 id, void *addr) {
    return syscall4(SYS_quotactl, cmd, special, id, addr);
}

static int wait_exit(i64 pid) {
    int status = 0;
    if (syscall4(SYS_wait4, pid, &status, 0, 0) != pid) return -1;
    return (status & 0x7f) ? 128 + (status & 0x7f) : (status >> 8) & 0xff;
}

static int in_private_ns(void (*body)(void)) {
    i64 pid = syscall0(SYS_fork);
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

#define N "/tmp/c_quotactl_blk_no_fs"

static void round(const char *tag) {
    char buf[128], n[64];
    str_cat2(n, tag, "_sync");
    expect(n, qc(QCMD(Q_SYNC, 0), N "/a", 0, 0), -ENODEV);
    str_cat2(n, tag, "_getfmt");
    expect(n, qc(QCMD(Q_GETFMT, 1), N "/a", 0, buf), -ENODEV);
    str_cat2(n, tag, "_getquota");
    expect(n, qc(QCMD(Q_GETQUOTA, 0), N "/b", 0, buf), -ENODEV);
    str_cat2(n, tag, "_quotaon_bad_addr");
    expect(n, qc(QCMD(Q_QUOTAON, 0), N "/a", QFMT_VFS_V1, (void *)1), -ENODEV);
    str_cat2(n, tag, "_unknown_cmd");
    expect(n, qc(QCMD(0x1234, 2), N "/b", 0, buf), -ENODEV);
    str_cat2(n, tag, "_bad_type");
    expect(n, qc(QCMD(Q_GETFMT, 3), N "/a", 0, buf), -EINVAL);
}

static void body(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    expect("mknod_a", syscall4(SYS_mknodat, AT_FDCWD, N "/a", S_IFBLK | 0600, NO_SUCH_DEV), 0);
    expect("mknod_b", syscall4(SYS_mknodat, AT_FDCWD, N "/b", S_IFBLK | 0600, NO_SUCH_DEV2), 0);
    round("root");
    expect("drop_caps", drop_caps(), 0);
    round("nocaps");
}

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR);
    expect("mkdir", syscall3(SYS_mkdirat, AT_FDCWD, N, 0755), 0);
    int code = in_private_ns(body);
    check("child_ok", code == 0, code);
    expect("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR), 0);
    return finish();
}
