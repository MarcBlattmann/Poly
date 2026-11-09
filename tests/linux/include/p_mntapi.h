/*
 * Helpers for the mount-API and pivot_root tests (pk_pivot_root_*,
 * pk_fsopen_*, pk_fsconfig_*, pk_fsmount_*, pk_fspick_*, pk_move_mount_*,
 * pk_open_tree_*, pk_mount_setattr_*). Include after a_sys.h.
 *
 * Constants and structs are ABI facts from the public Linux UAPI headers
 * (linux/mount.h, linux/sched.h, linux/fcntl.h, sys/statfs.h); syscall
 * numbers from asm/unistd_64.h. The helpers are written for Poly; no code
 * is copied from anywhere.
 *
 * Every test runs its whole body in a child that has a mount namespace of
 * its own (unshare(CLONE_NEWNS), "/" made MS_REC|MS_PRIVATE) and a tmpfs
 * on a private directory below /tmp, so nothing a test mounts is seen
 * outside and parallel tests cannot disturb each other. The parent only
 * makes and removes the (empty) mount point and adds up the checks.
 */
#ifndef P_MNTAPI_H
#define P_MNTAPI_H

#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_mknodat 259
#define SYS_pivot_root 155
#define SYS_chroot 161
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_unshare 272
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_prctl 157
#define SYS_open_tree 428
#define SYS_move_mount 429
#define SYS_fsopen 430
#define SYS_fsconfig 431
#define SYS_fsmount 432
#define SYS_fspick 433
#define SYS_mount_setattr 442

/* errno values (asm-generic/errno.h) not in a_sys.h */
#define ENODATA 61
#define EMSGSIZE 90

/* linux/sched.h */
#define CLONE_NEWNS 0x00020000
#define CLONE_NEWUSER 0x10000000

/* linux/mount.h (and sys/mount.h) */
#define MS_RDONLY 1
#define MS_NOSUID 2
#define MS_NODEV 4
#define MS_NOEXEC 8
#define MS_REMOUNT 32
#define MS_BIND 4096
#define MS_REC 16384
#define MS_UNBINDABLE (1 << 17)
#define MS_PRIVATE (1 << 18)
#define MS_SLAVE (1 << 19)
#define MS_SHARED (1 << 20)
#define MNT_DETACH 2

/* linux/mount.h: open_tree, move_mount, fsopen, fspick, fsmount */
#define OPEN_TREE_CLONE 1
#define OPEN_TREE_CLOEXEC O_CLOEXEC
#define AT_RECURSIVE 0x8000
#define MOVE_MOUNT_F_SYMLINKS 0x01
#define MOVE_MOUNT_F_AUTOMOUNTS 0x02
#define MOVE_MOUNT_F_EMPTY_PATH 0x04
#define MOVE_MOUNT_T_SYMLINKS 0x10
#define MOVE_MOUNT_T_AUTOMOUNTS 0x20
#define MOVE_MOUNT_T_EMPTY_PATH 0x40
#define MOVE_MOUNT_SET_GROUP 0x100
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
#define MOUNT_ATTR_IDMAP 0x100000
#define MOUNT_ATTR_NOSYMFOLLOW 0x200000

/* statfs(2) f_flags (sys/statvfs.h ST_*) */
#define ST_RDONLY 0x0001
#define ST_NOSUID 0x0002
#define ST_NODEV 0x0004
#define ST_NOEXEC 0x0008
#define ST_NOATIME 0x0400
#define ST_NODIRATIME 0x0800
#define ST_RELATIME 0x1000
#define ST_NOSYMFOLLOW 0x2000
#define TMPFS_MAGIC 0x01021994

/* linux/mount.h */
struct pm_mount_attr {
    u64 attr_set;
    u64 attr_clr;
    u64 propagation;
    u64 userns_fd;
};

static A_UNUSED i64 pm_mount(const char *src, const char *target, const char *type, u64 flags, const char *data) {
    return syscall5(SYS_mount, src, target, type, flags, data);
}
static A_UNUSED i64 pm_umount2(const char *target, i64 flags) { return syscall2(SYS_umount2, target, flags); }
static A_UNUSED i64 pm_pivot_root(const char *new_root, const char *put_old) {
    return syscall2(SYS_pivot_root, new_root, put_old);
}
static A_UNUSED i64 pm_open_tree(i64 dfd, const char *path, u64 flags) {
    return syscall3(SYS_open_tree, dfd, path, flags);
}
static A_UNUSED i64 pm_move_mount(i64 from_dfd, const char *from, i64 to_dfd, const char *to, u64 flags) {
    return syscall5(SYS_move_mount, from_dfd, from, to_dfd, to, flags);
}
static A_UNUSED i64 pm_fsopen(const char *fstype, u64 flags) { return syscall2(SYS_fsopen, fstype, flags); }
static A_UNUSED i64 pm_fsconfig(i64 fd, u64 cmd, const char *key, const void *value, i64 aux) {
    return syscall5(SYS_fsconfig, fd, cmd, key, value, aux);
}
static A_UNUSED i64 pm_fsmount(i64 fd, u64 flags, u64 attr) { return syscall3(SYS_fsmount, fd, flags, attr); }
static A_UNUSED i64 pm_fspick(i64 dfd, const char *path, u64 flags) { return syscall3(SYS_fspick, dfd, path, flags); }
static A_UNUSED i64 pm_mount_setattr(i64 dfd, const char *path, u64 flags, const void *attr, u64 size) {
    return syscall5(SYS_mount_setattr, dfd, path, flags, attr, size);
}

/* `base` + `rel` into `out` (rel without a leading slash; "" gives base). */
static A_UNUSED const char *pm_path(char *out, const char *base, const char *rel) {
    char *e = str_cpy(out, base);
    if (*rel) {
        *e++ = '/';
        str_cpy(e, rel);
    }
    return out;
}

/* mkdir -p style single level: ignore EEXIST. */
static A_UNUSED i64 pm_mkdir(const char *path) {
    i64 r = a_mkdir(path, 0755);
    return r == -EEXIST ? 0 : r;
}

/* Create an empty file; returns 0 or -errno. */
static A_UNUSED i64 pm_touch(const char *path) {
    i64 fd = a_open(path, O_WRONLY | O_CREAT, 0644);
    if (fd < 0) return fd;
    a_close(fd);
    return 0;
}

/* 0 when `path` exists, else -errno. */
static A_UNUSED i64 pm_exists(const char *path) { return syscall3(SYS_faccessat, AT_FDCWD, path, F_OK); }

/* The statfs f_flags of `path`, or -errno. */
static A_UNUSED i64 pm_statfs_flags(const char *path) {
    struct a_statfs s;
    i64 r = syscall2(SYS_statfs, path, &s);
    return r < 0 ? r : s.f_flags;
}

/* f_flags without the kernel-internal ST_VALID (0x20) bit. */
static A_UNUSED i64 pm_mntflags(const char *path) {
    i64 f = pm_statfs_flags(path);
    return f < 0 ? f : (f & ~0x20L);
}

/* f_blocks (in f_bsize units) of `path`, or -errno. */
static A_UNUSED i64 pm_statfs_blocks(const char *path) {
    struct a_statfs s;
    i64 r = syscall2(SYS_statfs, path, &s);
    return r < 0 ? r : (i64)s.f_blocks;
}

/* Can a regular file be created in dir `d`? 0 or -errno; removes it again. */
static A_UNUSED i64 pm_can_create(const char *dir) {
    char p[256];
    pm_path(p, dir, "pm_probe");
    i64 fd = a_open(p, O_WRONLY | O_CREAT | O_EXCL, 0644);
    if (fd < 0) return fd;
    a_close(fd);
    a_unlink(p);
    return 0;
}

/* One message of a filesystem context: returns its length and copies it
 * (NUL-terminated) to buf, or -errno. */
static A_UNUSED i64 pm_msg(i64 fd, char *buf, size_t size) {
    i64 n = a_read(fd, buf, size - 1);
    if (n >= 0) buf[n] = 0;
    return n;
}

/* Does the next message of context `fd` equal `want` (including its final
 * newline)? Returns 1 or 0; with a failing check the text is lost, so the
 * caller reports through check(). */
static A_UNUSED int pm_msg_is(i64 fd, const char *want) {
    char buf[256];
    i64 n = pm_msg(fd, buf, sizeof buf);
    return n == (i64)str_len(want) && str_eq(buf, want);
}

/* Run fn(arg) in a child that dropped to uid/gid 65534 without any
 * capability; returns what fn returned (an int, so 0 or -errno). */
static A_UNUSED i64 pm_as_nobody(i64 (*fn)(void *), void *arg) {
    volatile i32 *sh = a_shared();
    sh[8] = 12345;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall3(SYS_setresgid, 65534, 65534, 65534);
        syscall3(SYS_setresuid, 65534, 65534, 65534);
        sh[8] = (i32)fn(arg);
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    return sh[8];
}

/* Run fn(arg) in a child (fork semantics, the parent waits) that shares
 * this process's mount namespace; with `nobody` it first drops to uid/gid
 * 65534 without capabilities. The child's checks are added to this
 * process's totals. */
static A_UNUSED void pm_child(void (*fn)(void *), void *arg, int nobody) {
    volatile i32 *sh = a_shared();
    sh[2] = 0;
    sh[3] = 0;
    i64 pid = a_fork_sync();
    if (pid == 0) {
        if (nobody) {
            syscall3(SYS_setresgid, 65534, 65534, 65534);
            syscall3(SYS_setresuid, 65534, 65534, 65534);
        }
        fn(arg);
        sh[2] = passed;
        sh[3] = failed;
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("helper_child_exit_0", status, 0);
    passed += sh[2];
    failed += sh[3];
}

/* A new detached tmpfs: fsopen, optional "size=<size>", create, fsmount with
 * `attrs`. Returns the mount fd, or -errno of the first step that failed
 * (the context fd is closed). */
static A_UNUSED i64 pm_tmpfs_fd(const char *size, u64 attrs, u64 mount_flags) {
    i64 fc = pm_fsopen("tmpfs", FSOPEN_CLOEXEC);
    if (fc < 0) return fc;
    i64 r = 0;
    if (size) r = pm_fsconfig(fc, FSCONFIG_SET_STRING, "size", size, 0);
    if (r == 0) r = pm_fsconfig(fc, FSCONFIG_CMD_CREATE, 0, 0, 0);
    if (r == 0) r = pm_fsmount(fc, mount_flags, attrs);
    a_close(fc);
    return r;
}

/* Attach the detached mount `mfd` at `target`. */
static A_UNUSED i64 pm_attach(i64 mfd, const char *target) {
    return pm_move_mount(mfd, "", AT_FDCWD, target, MOVE_MOUNT_F_EMPTY_PATH);
}

/* fstatfs-like on a descriptor: f_flags or -errno. */
static A_UNUSED i64 pm_fd_statfs(i64 fd, struct a_statfs *s) { return syscall2(SYS_fstatfs, fd, s); }

/* Test body signature: gets its private directory (a tmpfs of its own). */
typedef void (*pm_body_fn)(const char *base);

/*
 * Run body(base) in a forked child with a mount namespace of its own and a
 * fresh tmpfs on `base` (below /tmp). Returns the test's exit status.
 */
static A_UNUSED int pm_main(const char *base, pm_body_fn body) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    syscall3(SYS_mkdirat, AT_FDCWD, base, 0755);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("unshare_newns", syscall1(SYS_unshare, CLONE_NEWNS), 0);
        expect("make_private", pm_mount(0, "/", 0, MS_REC | MS_PRIVATE, 0), 0);
        expect("mount_base", pm_mount("pk", base, "tmpfs", 0, "size=4m"), 0);
        body(base);
        sh[0] = passed;
        sh[1] = failed;
        syscall1(SYS_exit_group, 0);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_exit_0", status, 0);
    passed += sh[0];
    failed += sh[1];
    expect("rmdir_base", a_rmdir(base), 0);
    return finish();
}

#endif
