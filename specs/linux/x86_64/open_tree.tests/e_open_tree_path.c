/* open_tree(2) without OPEN_TREE_CLONE opens the path like
 * open(path, O_PATH): an O_PATH descriptor (close-on-exec only with
 * OPEN_TREE_CLOEXEC), usable as a dirfd and with fstat; no privilege is
 * needed. AT_SYMLINK_NOFOLLOW opens a symlink itself, AT_EMPTY_PATH with
 * "" reopens the dirfd. Errors: unknown flags EINVAL; AT_RECURSIVE without
 * OPEN_TREE_CLONE EINVAL; a missing path ENOENT; "" without AT_EMPTY_PATH
 * ENOENT; an unreadable path EFAULT; a closed dirfd EBADF.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_fstat 5
#define SYS_chdir 80
#define SYS_getcwd 79
#define SYS_statx 332
#define SYS_unshare 272
#define SYS_pivot_root 155
#define SYS_open_tree 428
#define SYS_move_mount 429
#define SYS_fsopen 430
#define SYS_fsconfig 431
#define SYS_fsmount 432
#define SYS_fspick 433
#define SYS_mount_setattr 442
#define SYS_statmount 457
#define SYS_listmount 458
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EBADF 9
#define EFAULT 14
#define EINVAL 22
#define E2BIG 7
#define EISDIR 21
#define ENODATA 61
#define EMSGSIZE 90
#define EOVERFLOW 75
/* linux/sched.h */
#define CLONE_NEWNS 0x00020000
/* asm-generic/fcntl.h, linux/fcntl.h */
#define O_PATH 010000000
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_NO_AUTOMOUNT 0x800
#define AT_EMPTY_PATH 0x1000
#define AT_RECURSIVE 0x8000
/* linux/mount.h */
#define OPEN_TREE_CLONE 1
#define OPEN_TREE_CLOEXEC O_CLOEXEC
#define MOVE_MOUNT_F_SYMLINKS 0x01
#define MOVE_MOUNT_F_AUTOMOUNTS 0x02
#define MOVE_MOUNT_F_EMPTY_PATH 0x04
#define MOVE_MOUNT_T_SYMLINKS 0x10
#define MOVE_MOUNT_T_AUTOMOUNTS 0x20
#define MOVE_MOUNT_T_EMPTY_PATH 0x40
#define FSOPEN_CLOEXEC 0x01
#define FSPICK_CLOEXEC 0x01
#define FSPICK_SYMLINK_NOFOLLOW 0x02
#define FSPICK_NO_AUTOMOUNT 0x04
#define FSPICK_EMPTY_PATH 0x08
#define FSCONFIG_SET_FLAG 0
#define FSCONFIG_SET_STRING 1
#define FSCONFIG_SET_BINARY 2
#define FSCONFIG_SET_PATH 3
#define FSCONFIG_SET_PATH_EMPTY 4
#define FSCONFIG_SET_FD 5
#define FSCONFIG_CMD_CREATE 6
#define FSCONFIG_CMD_RECONFIGURE 7
#define FSCONFIG_CMD_CREATE_EXCL 8
#define FSMOUNT_CLOEXEC 0x01
#define MOUNT_ATTR_RDONLY 0x01
#define MOUNT_ATTR_NOSUID 0x02
#define MOUNT_ATTR_NODEV 0x04
#define MOUNT_ATTR_NOEXEC 0x08
#define MOUNT_ATTR__ATIME 0x70
#define MOUNT_ATTR_RELATIME 0x00
#define MOUNT_ATTR_NOATIME 0x10
#define MOUNT_ATTR_STRICTATIME 0x20
#define MOUNT_ATTR_NODIRATIME 0x80
#define MOUNT_ATTR_NOSYMFOLLOW 0x200000
#define MOUNT_ATTR_SIZE_VER0 32
#define MS_SHARED (1 << 20)
#define MS_SLAVE (1 << 19)
#define MS_UNBINDABLE (1 << 17)
#define STATMOUNT_SB_BASIC 0x1
#define STATMOUNT_MNT_BASIC 0x2
#define STATMOUNT_PROPAGATE_FROM 0x4
#define STATMOUNT_MNT_ROOT 0x8
#define STATMOUNT_MNT_POINT 0x10
#define STATMOUNT_FS_TYPE 0x20
#define STATMOUNT_MNT_NS_ID 0x40
#define STATMOUNT_MNT_OPTS 0x80
#define STATMOUNT_SB_SOURCE 0x200
#define LSMT_ROOT 0xffffffffffffffffUL
#define LISTMOUNT_REVERSE 1
#define MNT_ID_REQ_SIZE_VER0 24
/* statfs f_flags (ST_* in sys/statvfs.h, as Linux reports them) */
#define ST_RDONLY 1
#define ST_NOSUID 2
#define ST_NODEV 4
#define ST_NOEXEC 8
#define ST_NOATIME 1024
#define ST_NODIRATIME 2048
#define ST_RELATIME 4096
#define ST_NOSYMFOLLOW 0x2000
/* linux/stat.h */
#define STATX_MNT_ID_UNIQUE 0x4000

struct k_mount_attr {
    u64 attr_set, attr_clr, propagation, userns_fd;
};
struct k_mnt_id_req {
    unsigned size, spare;
    u64 mnt_id, param, mnt_ns_id;
};
/* struct statmount (linux/mount.h), the fields up to the string area. */
struct k_statmount {
    unsigned size, mnt_opts;
    u64 mask;
    unsigned sb_dev_major, sb_dev_minor;
    u64 sb_magic;
    unsigned sb_flags, fs_type;
    u64 mnt_id, mnt_parent_id;
    unsigned mnt_id_old, mnt_parent_id_old;
    u64 mnt_attr, mnt_propagation, mnt_peer_group, mnt_master, propagate_from;
    unsigned mnt_root, mnt_point;
    u64 mnt_ns_id;
    unsigned fs_subtype, sb_source, opt_num, opt_array, opt_sec_num, opt_sec_array;
    u64 supported_mask;
    unsigned mnt_uidmap_num, mnt_gidmap_num, mnt_uidmap, mnt_gidmap;
    u64 spare2[43];
    char str[];
};
struct k_statfs {
    i64 f_type, f_bsize;
    u64 f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int f_fsid[2];
    i64 f_namelen, f_frsize, f_flags, f_spare[4];
};
struct k_statx {
    unsigned mask, blksize;
    u64 attributes;
    unsigned nlink, uid, gid;
    unsigned short mode, pad1;
    u64 ino, size, blocks, attributes_mask;
    struct {
        i64 sec;
        unsigned nsec;
        int pad;
    } atime, btime, ctime, mtime;
    unsigned rdev_major, rdev_minor, dev_major, dev_minor;
    u64 mnt_id;
    unsigned dio_mem_align, dio_offset_align;
    u64 rest[12];
};

static __attribute__((unused)) i64 do_mount(const char *src, const char *dst, const char *type, u64 flags,
                                            const char *data) {
    return syscall5(SYS_mount, src, dst, type, flags, data);
}
static __attribute__((unused)) i64 umount_detach(const char *p) { return syscall2(SYS_umount2, p, MNT_DETACH); }
static __attribute__((unused)) i64 mkdir_p(const char *p) { return syscall3(SYS_mkdirat, AT_FDCWD, p, 0755); }
static __attribute__((unused)) i64 rmdir_p(const char *p) { return syscall3(SYS_unlinkat, AT_FDCWD, p, AT_REMOVEDIR); }
static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 fsopen(const char *fs, unsigned flags) { return syscall2(SYS_fsopen, fs, flags); }
static __attribute__((unused)) i64 fsconfig(int fd, unsigned cmd, const char *key, const void *val, int aux) {
    return syscall5(SYS_fsconfig, fd, cmd, key, val, aux);
}
static __attribute__((unused)) i64 fsmount(int fd, unsigned flags, unsigned attrs) {
    return syscall3(SYS_fsmount, fd, flags, attrs);
}
static __attribute__((unused)) i64 fspick(int dfd, const char *path, unsigned flags) {
    return syscall3(SYS_fspick, dfd, path, flags);
}
static __attribute__((unused)) i64 open_tree(int dfd, const char *path, unsigned flags) {
    return syscall3(SYS_open_tree, dfd, path, flags);
}
static __attribute__((unused)) i64 move_mount(int fdfd, const char *fp, int tdfd, const char *tp, unsigned flags) {
    return syscall5(SYS_move_mount, fdfd, fp, tdfd, tp, flags);
}
static __attribute__((unused)) i64 mount_setattr(int dfd, const char *path, unsigned flags, struct k_mount_attr *a,
                                                 u64 size) {
    return syscall5(SYS_mount_setattr, dfd, path, flags, a, size);
}
static __attribute__((unused)) i64 statmount(struct k_mnt_id_req *r, void *buf, u64 size, unsigned flags) {
    return syscall4(SYS_statmount, r, buf, size, flags);
}
static __attribute__((unused)) i64 listmount(struct k_mnt_id_req *r, u64 *ids, u64 nr, unsigned flags) {
    return syscall4(SYS_listmount, r, ids, nr, flags);
}
/* The unique 64-bit mount id of the mount `path` is on (statx). */
static __attribute__((unused)) u64 mnt_id_of(const char *path) {
    struct k_statx sx;
    memset(&sx, 0, sizeof sx);
    if (syscall5(SYS_statx, AT_FDCWD, path, 0, STATX_MNT_ID_UNIQUE, &sx) != 0) return 0;
    if (!(sx.mask & STATX_MNT_ID_UNIQUE)) return 0;
    return sx.mnt_id;
}
static __attribute__((unused)) i64 statfs_flags(const char *path) {
    struct k_statfs sf;
    memset(&sf, 0, sizeof sf);
    if (syscall2(SYS_statfs, path, &sf) != 0) return -1;
    return sf.f_flags;
}
static __attribute__((unused)) int can_create(const char *path) {
    i64 fd = open3(path, O_CREAT | O_RDWR, 0644);
    if (fd < 0) return (int)fd;
    close((int)fd);
    syscall3(SYS_unlinkat, AT_FDCWD, path, 0);
    return 1;
}
static __attribute__((unused)) int exists(const char *path) {
    struct k_statx sx;
    return syscall5(SYS_statx, AT_FDCWD, path, AT_SYMLINK_NOFOLLOW, 0, &sx) == 0;
}
/* Remove every mount stacked on `p` (up to 4). */
static __attribute__((unused)) void clear_mounts(const char *p) {
    for (int i = 0; i < 4; i++) umount_detach(p);
}
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
/* A ready tmpfs fs context (fsopen + options + CMD_CREATE); -errno. */
static __attribute__((unused)) i64 tmpfs_context(const char *size) {
    i64 fc = fsopen("tmpfs", FSOPEN_CLOEXEC);
    if (fc < 0) return fc;
    if (size) fsconfig((int)fc, FSCONFIG_SET_STRING, "size", size, 0);
    i64 r = fsconfig((int)fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    if (r < 0) {
        close((int)fc);
        return r;
    }
    return fc;
}

#define D "/tmp/e_open_tree_path"

int test_main(void) {
    syscall3(SYS_unlinkat, AT_FDCWD, D "/link", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/file", 0);
    rmdir_p(D);
    mkdir_p(D);
    i64 f = open3(D "/file", O_CREAT | O_RDWR, 0644);
    close((int)f);
    syscall3(SYS_symlinkat, "file", AT_FDCWD, D "/link");

    i64 fd = open_tree(AT_FDCWD, D, 0);
    check("fd", fd == 3, fd);
    check("o_path", syscall2(SYS_fcntl, fd, F_GETFL) == O_PATH, syscall2(SYS_fcntl, fd, F_GETFL));
    check("no_cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == 0, 0);
    i64 sub = syscall4(SYS_openat, fd, "file", O_RDONLY, 0);
    check("usable_as_dirfd", sub == 4, sub);
    close((int)sub);
    i64 c = open_tree(AT_FDCWD, D, OPEN_TREE_CLOEXEC);
    check("cloexec", c == 4 && syscall2(SYS_fcntl, c, F_GETFD) == FD_CLOEXEC, c);
    close((int)c);
    struct k_statx sx;
    i64 l = open_tree(AT_FDCWD, D "/link", AT_SYMLINK_NOFOLLOW);
    syscall5(SYS_statx, l, "", AT_EMPTY_PATH, 0x2, &sx);
    check("nofollow_symlink", (sx.mode & 0170000) == 0120000, sx.mode);
    close((int)l);
    l = open_tree(AT_FDCWD, D "/link", 0);
    syscall5(SYS_statx, l, "", AT_EMPTY_PATH, 0x2, &sx);
    check("follows_symlink", (sx.mode & 0170000) == 0100000, sx.mode);
    close((int)l);
    i64 e = open_tree((int)fd, "", AT_EMPTY_PATH);
    syscall5(SYS_statx, e, "", AT_EMPTY_PATH, 0x2, &sx);
    check("empty_path_reopens", e >= 0 && (sx.mode & 0170000) == 040000, e);
    close((int)e);

    i64 r = open_tree(AT_FDCWD, D, 0x2);
    check("bad_flags_einval", r == -EINVAL, r);
    r = open_tree(AT_FDCWD, D, AT_RECURSIVE);
    check("recursive_without_clone_einval", r == -EINVAL, r);
    r = open_tree(AT_FDCWD, D "/none", 0);
    check("missing_enoent", r == -ENOENT, r);
    r = open_tree((int)fd, "", 0);
    check("empty_without_flag_enoent", r == -ENOENT, r);
    r = open_tree(AT_FDCWD, (const char *)8, 0);
    check("path_efault", r == -EFAULT, r);
    r = open_tree(99, "file", 0);
    check("closed_dirfd_ebadf", r == -EBADF, r);
    close((int)fd);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/link", 0);
    syscall3(SYS_unlinkat, AT_FDCWD, D "/file", 0);
    rmdir_p(D);
    return finish();
}
