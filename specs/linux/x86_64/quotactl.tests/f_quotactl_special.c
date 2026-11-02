/* quotactl(2) finds the file system through its special file, which must
 * be a block device with a mounted file system. Any command other than
 * Q_SYNC with a NULL special gives ENODEV (Q_GETFMT, Q_QUOTAOFF,
 * Q_GETQUOTA, Q_XGETQSTAT). A special path is resolved like any path:
 * missing or empty ENOENT, through a non-directory ENOTDIR, unreadable
 * EFAULT; a directory or a character device gives ENOTBLK (whatever the
 * command: Q_GETFMT, Q_QUOTAON, Q_GETQUOTA, Q_XGETQSTAT). These results
 * are the same for every user.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_pwrite64 18
#define SYS_geteuid 107
#define SYS_swapon 167
#define SYS_swapoff 168
#define SYS_reboot 169
#define SYS_init_module 175
#define SYS_delete_module 176
#define SYS_quotactl 179
#define SYS_unshare 272
#define SYS_finit_module 313
#define SYS_memfd_create 319
#define SYS_open_tree 428
#define SYS_move_mount 429
#define SYS_quotactl_fd 443
#define SYS_open_tree_attr 467
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define ENOEXEC 8
#define ENOTBLK 15
#define ETXTBSY 26
#define EISDIR 21
/* linux/sched.h */
#define CLONE_NEWNS 0x00020000
#define CLONE_NEWPID 0x20000000
/* linux/reboot.h */
#define LINUX_REBOOT_MAGIC1 0xfee1dead
#define LINUX_REBOOT_MAGIC2 672274793
#define LINUX_REBOOT_MAGIC2A 85072278
#define LINUX_REBOOT_MAGIC2B 369367448
#define LINUX_REBOOT_MAGIC2C 537993216
#define LINUX_REBOOT_CMD_RESTART 0x01234567
#define LINUX_REBOOT_CMD_HALT 0xCDEF0123
#define LINUX_REBOOT_CMD_CAD_ON 0x89ABCDEF
#define LINUX_REBOOT_CMD_CAD_OFF 0x00000000
#define LINUX_REBOOT_CMD_POWER_OFF 0x4321FEDC
#define LINUX_REBOOT_CMD_RESTART2 0xA1B2C3D4
#define LINUX_REBOOT_CMD_SW_SUSPEND 0xD000FCE2
#define LINUX_REBOOT_CMD_KEXEC 0x45584543
/* linux/swap.h */
#define SWAP_FLAG_PREFER 0x8000
#define SWAP_FLAG_PRIO_MASK 0x7fff
#define SWAP_FLAG_DISCARD 0x10000
#define SWAP_FLAG_DISCARD_ONCE 0x20000
#define SWAP_FLAG_DISCARD_PAGES 0x40000
/* linux/quota.h */
#define SUBCMDSHIFT 8
#define QCMD(cmd, type) (((cmd) << SUBCMDSHIFT) | ((type) & 0xff))
#define Q_SYNC 0x800001
#define Q_QUOTAON 0x800002
#define Q_QUOTAOFF 0x800003
#define Q_GETFMT 0x800004
#define Q_GETINFO 0x800005
#define Q_SETINFO 0x800006
#define Q_GETQUOTA 0x800007
#define Q_SETQUOTA 0x800008
#define Q_GETNEXTQUOTA 0x800009
#define USRQUOTA 0
#define GRPQUOTA 1
#define PRJQUOTA 2
#define QFMT_VFS_V1 4
/* linux/dqblk_xfs.h */
#define Q_XGETQSTAT 0x5805
#define Q_XQUOTASYNC 0x5807
/* linux/module.h */
#define MODULE_INIT_IGNORE_MODVERSIONS 1
#define MODULE_INIT_IGNORE_VERMAGIC 2
#define MODULE_INIT_COMPRESSED_FILE 4
/* asm-generic/fcntl.h, linux/fcntl.h */
#define O_PATH 010000000
#define O_APPEND 02000
#define AT_EMPTY_PATH 0x1000
#define AT_RECURSIVE 0x8000
/* linux/mount.h */
#define OPEN_TREE_CLONE 1
#define OPEN_TREE_CLOEXEC O_CLOEXEC
#define MOVE_MOUNT_F_EMPTY_PATH 0x04
#define MOUNT_ATTR_RDONLY 0x01
#define MOUNT_ATTR_NOSUID 0x02
#define MOUNT_ATTR_NODEV 0x04
#define MOUNT_ATTR_NOEXEC 0x08
#define MOUNT_ATTR__ATIME 0x70
#define MOUNT_ATTR_NOATIME 0x10
#define MOUNT_ATTR_IDMAP 0x00100000
#define MOUNT_ATTR_SIZE_VER0 32
#define MS_SHARED (1 << 20)
/* statfs f_flags (ST_* in sys/statvfs.h, as Linux reports them) */
#define ST_RDONLY 1
#define ST_NOSUID 2
#define ST_NOATIME 1024
#define ST_RELATIME 4096
/* asm-generic/mman-common.h */
#define PROT_NONE 0x0

struct k_mount_attr {
    u64 attr_set, attr_clr, propagation, userns_fd;
};
struct k_statfs {
    i64 f_type, f_bsize;
    u64 f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int f_fsid[2];
    i64 f_namelen, f_frsize, f_flags, f_spare[4];
};

static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
static __attribute__((unused)) i64 mkdir_p(const char *p) { return syscall3(SYS_mkdirat, AT_FDCWD, p, 0755); }
static __attribute__((unused)) i64 rmdir_p(const char *p) { return syscall3(SYS_unlinkat, AT_FDCWD, p, AT_REMOVEDIR); }
/* A file of `len` bytes, all `fill`, with `text` (if any) at offset `at`. */
static __attribute__((unused)) void make_file(const char *path, u64 len, int fill, const char *text, u64 at) {
    static char block[4096];
    unlink_path(path);
    i64 fd = open3(path, O_CREAT | O_RDWR | O_EXCL, 0600);
    if (fd < 0) return;
    for (u64 off = 0; off < len; off += sizeof block) {
        u64 n = len - off < sizeof block ? len - off : sizeof block;
        memset(block, fill, sizeof block);
        if (text && at >= off && at < off + n) memcpy(block + (at - off), text, str_len(text));
        write((int)fd, block, n);
    }
    close((int)fd);
}
static __attribute__((unused)) i64 geteuid(void) { return syscall0(SYS_geteuid); }
static __attribute__((unused)) i64 fork_sync(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGCHLD), 0);
    return syscall5(SYS_clone, 0x4000 /* CLONE_VFORK */ | SIGCHLD, 0, 0, 0, 0);
}
/* Become uid/gid 1000 with no capabilities (in a child). */
static __attribute__((unused)) int drop_to_user(void) {
    if (syscall3(SYS_setresgid, 1000, 1000, 1000) != 0) return 0;
    if (syscall3(SYS_setresuid, 1000, 1000, 1000) != 0) return 0;
    return 1;
}
/* In a child: lose root if the test runs as root; a test already run by
 * another user keeps that user. 0 if root could not be left. */
static __attribute__((unused)) int become_unprivileged(void) {
    if (geteuid() != 0) return 1;
    return drop_to_user();
}
static __attribute__((unused)) i64 do_mount(const char *src, const char *dst, const char *type, u64 flags,
                                            const char *data) {
    return syscall5(SYS_mount, src, dst, type, flags, data);
}
static __attribute__((unused)) i64 umount_detach(const char *p) { return syscall2(SYS_umount2, p, MNT_DETACH); }
/* Remove every mount stacked on `p` (up to 4). */
static __attribute__((unused)) void clear_mounts(const char *p) {
    for (int i = 0; i < 4; i++) umount_detach(p);
}
static __attribute__((unused)) i64 open_tree_attr(int dfd, const char *path, unsigned flags, void *attr, u64 size) {
    return syscall5(SYS_open_tree_attr, dfd, path, flags, attr, size);
}
static __attribute__((unused)) i64 move_mount_fd(i64 fd, const char *to) {
    return syscall5(SYS_move_mount, fd, "", AT_FDCWD, to, MOVE_MOUNT_F_EMPTY_PATH);
}
static __attribute__((unused)) i64 statfs_flags(const char *path) {
    struct k_statfs sf;
    memset(&sf, 0, sizeof sf);
    if (syscall2(SYS_statfs, path, &sf) != 0) return -1;
    return sf.f_flags;
}
/* 1 if a file can be created (and is removed again) at path, else -errno. */
static __attribute__((unused)) int can_create(const char *path) {
    i64 fd = open3(path, O_CREAT | O_RDWR, 0644);
    if (fd < 0) return (int)fd;
    close((int)fd);
    unlink_path(path);
    return 1;
}
static __attribute__((unused)) void *map_anon(u64 len, int prot) {
    return (void *)syscall6(SYS_mmap, 0, (i64)len, prot, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
}

int test_main(void) {
    char buf[256];
    i64 r;
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), 0, 0, buf);
    check("getfmt_null_enodev", r == -ENODEV, r);
    r = syscall4(SYS_quotactl, QCMD(Q_QUOTAOFF, USRQUOTA), 0, 0, 0);
    check("quotaoff_null_enodev", r == -ENODEV, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETQUOTA, GRPQUOTA), 0, 0, buf);
    check("getquota_null_enodev", r == -ENODEV, r);
    r = syscall4(SYS_quotactl, QCMD(Q_XGETQSTAT, USRQUOTA), 0, 0, buf);
    check("xgetqstat_null_enodev", r == -ENODEV, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), "/dev/shm/f_quotactl_special_none", 0, buf);
    check("missing_enoent", r == -ENOENT, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), "", 0, buf);
    check("empty_enoent", r == -ENOENT, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), "/dev/null/x", 0, buf);
    check("through_chardev_enotdir", r == -ENOTDIR, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), 8, 0, buf);
    check("bad_pointer_efault", r == -EFAULT, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), "/dev/shm", 0, buf);
    check("getfmt_directory_enotblk", r == -ENOTBLK, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETFMT, USRQUOTA), "/dev/null", 0, buf);
    check("getfmt_chardev_enotblk", r == -ENOTBLK, r);
    r = syscall4(SYS_quotactl, QCMD(Q_QUOTAON, USRQUOTA), "/dev/shm", QFMT_VFS_V1, "/x");
    check("quotaon_directory_enotblk", r == -ENOTBLK, r);
    r = syscall4(SYS_quotactl, QCMD(Q_GETQUOTA, USRQUOTA), "/dev/shm", 0, buf);
    check("getquota_directory_enotblk", r == -ENOTBLK, r);
    r = syscall4(SYS_quotactl, QCMD(Q_XGETQSTAT, USRQUOTA), "/dev/shm", 0, buf);
    check("xgetqstat_directory_enotblk", r == -ENOTBLK, r);
    return finish();
}
