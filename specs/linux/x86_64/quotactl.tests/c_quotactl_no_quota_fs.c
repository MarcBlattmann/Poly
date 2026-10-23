/* A block device node carrying the device number (st_dev) of a mounted
 * file system that has no quota support at all (procfs; a tmpfs mounted
 * without quota options) is ENOSYS (38), for every subcommand, known or
 * unknown, and with all capabilities dropped too: that is decided before
 * any privilege check. A bad type is still EINVAL first.
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
#define Q_QUOTAOFF 0x800003
#define Q_GETFMT 0x800004
#define Q_GETINFO 0x800005
#define Q_GETQUOTA 0x800007
#define Q_SETQUOTA 0x800008

struct k_stat {
    u64 dev, ino, nlink;
    unsigned mode, uid, gid, pad0;
    u64 rdev;
    i64 size, blksize, blocks;
    u64 times[6];
    i64 unused[3];
};

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

/* st_dev uses the same encoding as mknod's dev argument. */
static u64 dev_of(const char *path) {
    struct k_stat st;
    if (syscall4(SYS_newfstatat, AT_FDCWD, path, &st, 0) != 0) return 0;
    return st.dev;
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

#define N "/tmp/c_quotactl_no_quota_fs"

static void round(const char *tag, const char *node) {
    char buf[128], n[64];
    memset(buf, 0, sizeof buf);
    i64 cmds[7] = { QCMD(Q_SYNC, 0),     QCMD(Q_GETFMT, 1),   QCMD(Q_GETINFO, 2), QCMD(Q_GETQUOTA, 0),
                    QCMD(Q_SETQUOTA, 1), QCMD(Q_QUOTAOFF, 0), QCMD(0x1234, 2) };
    const char *names[7] = { "_sync", "_getfmt", "_getinfo", "_getquota", "_setquota", "_quotaoff", "_unknown" };
    for (int i = 0; i < 7; i++) {
        str_cat2(n, tag, names[i]);
        expect(n, qc(cmds[i], node, 0, buf), -ENOSYS);
    }
    str_cat2(n, tag, "_bad_type");
    expect(n, qc(QCMD(Q_GETFMT, 3), node, 0, buf), -EINVAL);
}

static void body(void) {
    i64 r = syscall5(SYS_mount, "x", N, "tmpfs", 0, 0);
    check("mount_tmpfs", r == 0, r);
    if (r != 0) return;
    u64 proc = dev_of("/proc/self"), tmp = dev_of(N);
    check("proc_dev", proc != 0, 0);
    check("tmpfs_dev", tmp != 0 && tmp != proc, 0);
    expect("mknod_proc", syscall4(SYS_mknodat, AT_FDCWD, N "/proc", S_IFBLK | 0600, proc), 0);
    expect("mknod_tmpfs", syscall4(SYS_mknodat, AT_FDCWD, N "/tmpfs", S_IFBLK | 0600, tmp), 0);
    round("proc_root", N "/proc");
    round("tmpfs_root", N "/tmpfs");
    expect("drop_caps", drop_caps(), 0);
    round("proc_nocaps", N "/proc");
    round("tmpfs_nocaps", N "/tmpfs");
}

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR);
    expect("mkdir", syscall3(SYS_mkdirat, AT_FDCWD, N, 0755), 0);
    int code = in_private_ns(body);
    check("child_ok", code == 0, code);
    expect("cleanup", syscall3(SYS_unlinkat, AT_FDCWD, N, AT_REMOVEDIR), 0);
    return finish();
}
