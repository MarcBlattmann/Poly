/*
 * Helpers for the loop device and squashfs tests (pk_loop_*, pk_squashfs_*).
 * Include after a_sys.h. Constants and struct layouts: linux/loop.h,
 * linux/major.h, linux/fs.h, linux/mount.h (public UAPI headers). Written
 * for Poly; behaviour only.
 *
 * Each test works in a private directory under /tmp: it makes its own
 * device nodes (loop-control is character device 10:237, loop devices are
 * block devices with major 7), so nothing depends on /dev. Loop devices
 * come from LOOP_CTL_GET_FREE and are retried on EBUSY (the kernel is shared
 * with other users of loop devices); device numbers are never printed.
 */
#ifndef P_LOOP_H
#define P_LOOP_H

#include "a_sys.h"

#ifndef ENODATA
#define ENODATA 61 /* asm-generic/errno.h */
#endif
#ifndef ENOTBLK
#define ENOTBLK 15 /* asm-generic/errno-base.h */
#endif
#define SYS_ioctl 16
#define SYS_mknod 133
#define SYS_mount 165
#define SYS_umount2 166
#define SYS_getxattr 191
#define SYS_listxattr 194

/* linux/loop.h */
#define LOOP_SET_FD 0x4C00
#define LOOP_CLR_FD 0x4C01
#define LOOP_SET_STATUS 0x4C02
#define LOOP_GET_STATUS 0x4C03
#define LOOP_SET_STATUS64 0x4C04
#define LOOP_GET_STATUS64 0x4C05
#define LOOP_CHANGE_FD 0x4C06
#define LOOP_SET_CAPACITY 0x4C07
#define LOOP_SET_DIRECT_IO 0x4C08
#define LOOP_SET_BLOCK_SIZE 0x4C09
#define LOOP_CONFIGURE 0x4C0A
#define LOOP_CTL_ADD 0x4C80
#define LOOP_CTL_REMOVE 0x4C81
#define LOOP_CTL_GET_FREE 0x4C82
#define LO_FLAGS_READ_ONLY 1
#define LO_FLAGS_AUTOCLEAR 4
#define LO_FLAGS_PARTSCAN 8
#define LO_FLAGS_DIRECT_IO 16
#define LOOP_MAJOR 7

/* linux/fs.h */
#define BLKROSET 0x125D
#define BLKROGET 0x125E
#define BLKGETSIZE 0x1260
#define BLKFLSBUF 0x1261
#define BLKSSZGET 0x1268
#define BLKGETSIZE64 0x80081272

/* linux/mount.h */
#define MS_RDONLY 1
#define MS_REMOUNT 32
#define MNT_DETACH 2

struct p_loop_info64 {
    u64 lo_device, lo_inode, lo_rdevice, lo_offset, lo_sizelimit;
    u32 lo_number, lo_encrypt_type, lo_encrypt_key_size, lo_flags;
    u8 lo_file_name[64], lo_crypt_name[64], lo_encrypt_key[32];
    u64 lo_init[2];
};

struct p_loop_config {
    u32 fd, block_size;
    struct p_loop_info64 info;
    u64 reserved[8];
};

static A_UNUSED u64 p_dev(u64 major, u64 minor) {
    return (minor & 0xff) | (major << 8) | ((minor & ~0xffUL) << 12);
}

static A_UNUSED i64 p_ioctl(i64 fd, u64 cmd, i64 arg) { return syscall3(SYS_ioctl, fd, cmd, arg); }

/* Create (or recreate) the private directory. */
static A_UNUSED i64 p_mkdir(const char *dir) {
    a_rmdir(dir);
    return a_mkdir(dir, 0755);
}

/* The loop-control descriptor of the last p_ctl_open (used by p_detach to remove devices). */
static A_UNUSED i64 p_ctl_fd = -1;

/* Make loop-control at `path` and open it. */
static A_UNUSED i64 p_ctl_open(const char *path) {
    a_unlink(path);
    i64 r = syscall3(SYS_mknod, path, S_IFCHR | 0600, p_dev(10, 237));
    if (r < 0) return r;
    p_ctl_fd = a_open(path, O_RDWR, 0);
    return p_ctl_fd;
}

/*
 * Devices made by the tests get numbers from 600000 up (LOOP_CTL_ADD with that number), so
 * that LOOP_CTL_GET_FREE, which hands out the lowest unbound device to everybody on the
 * machine, never returns a device a test is still using while it is unbound.
 */
#define P_LOOP_BASE 600000
static A_UNUSED i64 p_next_dev;

static A_UNUSED i64 p_added[64];
static A_UNUSED int p_added_n;

/* Add a loop device with a number of our own; returns the number or -errno. */
static A_UNUSED i64 p_add(i64 ctl) {
    if (!p_next_dev) p_next_dev = P_LOOP_BASE + syscall0(SYS_getpid) % 300000;
    for (int tries = 0; tries < 1000; tries++) {
        i64 n = p_next_dev++;
        if (p_next_dev >= 0xfffff) p_next_dev = P_LOOP_BASE;
        i64 r = p_ioctl(ctl, LOOP_CTL_ADD, n);
        if (r == n) {
            if (p_added_n < 64) p_added[p_added_n++] = n;
            return n;
        }
        if (r != -EEXIST) return r;
    }
    return -EEXIST;
}

/* Make the block device node for loop device `n` at `path`. */
static A_UNUSED i64 p_node(const char *path, i64 n) {
    a_unlink(path);
    return syscall3(SYS_mknod, path, S_IFBLK | 0600, p_dev(LOOP_MAJOR, (u64)n));
}

/* Create `path` with `len` bytes of `data`. Returns an fd open O_RDWR or -errno. */
static A_UNUSED i64 p_file(const char *path, const void *data, size_t len) {
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) return fd;
    size_t off = 0;
    while (off < len) {
        i64 w = a_write(fd, (const char *)data + off, len - off);
        if (w <= 0) return w < 0 ? w : -EIO;
        off += (size_t)w;
    }
    return fd;
}

/*
 * Create `path` holding `prefix` bytes of 0x5a, then the image `img` of `len` bytes padded
 * with zeros to a multiple of 4096 (as mksquashfs pads its output). Returns an fd open
 * O_RDWR or -errno.
 */
static A_UNUSED char p_image_buf[1 << 17];
static A_UNUSED i64 p_image(const char *path, const unsigned char *img, size_t len, size_t prefix) {
    size_t total = prefix + len;
    total = (total + 4095) & ~(size_t)4095;
    if (total > sizeof p_image_buf) return -EFBIG;
    memset(p_image_buf, 0, total);
    memset(p_image_buf, 0x5a, prefix);
    memcpy(p_image_buf + prefix, img, len);
    return p_file(path, p_image_buf, total);
}

/*
 * Attach `backing_fd` to a free loop device (LOOP_CTL_GET_FREE, then
 * LOOP_SET_FD, or LOOP_CONFIGURE with `info` when `use_config` is set;
 * retried while the device is taken). Makes the node at `node`, returns the
 * open device fd (O_RDWR) or -errno; *num gets the device number.
 */
static A_UNUSED i64 p_attach_free(i64 ctl, i64 backing_fd, const char *node, i64 *num, const struct p_loop_info64 *info) {
    for (int tries = 0; tries < 200; tries++) {
        i64 n = p_ioctl(ctl, LOOP_CTL_GET_FREE, 0);
        if (n < 0) return n;
        i64 r = p_node(node, n);
        if (r < 0) return r;
        i64 fd = a_open(node, O_RDWR, 0);
        if (fd < 0) return fd;
        if (info) {
            struct p_loop_config cfg;
            memset(&cfg, 0, sizeof cfg);
            cfg.fd = (u32)backing_fd;
            cfg.info = *info;
            r = p_ioctl(fd, LOOP_CONFIGURE, (i64)&cfg);
        } else {
            r = p_ioctl(fd, LOOP_SET_FD, backing_fd);
        }
        if (r == 0) {
            if (num) *num = n;
            return fd;
        }
        a_close(fd);
        if (r != -EBUSY) return r;
    }
    return -EBUSY;
}

/* Same as p_attach_free but on a device of our own (see P_LOOP_BASE), which p_detach removes. */
static A_UNUSED i64 p_attach(i64 ctl, i64 backing_fd, const char *node, i64 *num, const struct p_loop_info64 *info) {
    i64 n = p_add(ctl);
    if (n < 0) return n;
    i64 r = p_node(node, n);
    if (r < 0) return r;
    i64 fd = a_open(node, O_RDWR, 0);
    if (fd < 0) return fd;
    if (info) {
        struct p_loop_config cfg;
        memset(&cfg, 0, sizeof cfg);
        cfg.fd = (u32)backing_fd;
        cfg.info = *info;
        r = p_ioctl(fd, LOOP_CONFIGURE, (i64)&cfg);
    } else {
        r = p_ioctl(fd, LOOP_SET_FD, backing_fd);
    }
    if (r < 0) {
        a_close(fd);
        p_ioctl(ctl, LOOP_CTL_REMOVE, n);
        return r;
    }
    if (num) *num = n;
    return fd;
}

/* Unbind, close, and remove the device if it is one of ours (number >= P_LOOP_BASE). */
static A_UNUSED void p_detach(i64 devfd) {
    struct a_stat st;
    i64 n = -1;
    if (syscall2(SYS_fstat, devfd, &st) == 0) n = (i64)((st.st_rdev & 0xff) | ((st.st_rdev >> 12) & ~0xffUL));
    p_ioctl(devfd, LOOP_CLR_FD, 0);
    a_close(devfd);
    if (n >= P_LOOP_BASE && p_ctl_fd >= 0) p_ioctl(p_ctl_fd, LOOP_CTL_REMOVE, n);
}

static A_UNUSED i64 p_mount_squash(const char *dev, const char *target, u64 flags) {
    return syscall5(SYS_mount, dev, target, "squashfs", flags | MS_RDONLY, 0);
}

static A_UNUSED i64 p_umount(const char *target) { return syscall2(SYS_umount2, target, 0); }

/*
 * One squashfs image in a private directory `dir`: dir/ctl (loop-control), dir/img (the image
 * file, padded), dir/l (the loop device node), dir/m (the mount point).
 */
struct p_sq {
    i64 ctl, dev, num;
    char dir[64], ctlp[80], imgp[80], node[80], mnt[80];
};

/* When set, p_sq_setup binds with LOOP_CONFIGURE and this status instead of LOOP_SET_FD. */
static A_UNUSED const struct p_loop_info64 *p_sq_cfg;

static A_UNUSED void p_sq_paths(struct p_sq *s, const char *dir) {
    str_cpy(s->dir, dir);
    str_cat2(s->ctlp, dir, "/ctl");
    str_cat2(s->imgp, dir, "/img");
    str_cat2(s->node, dir, "/l");
    str_cat2(s->mnt, dir, "/m");
}

/* Make the directory, the image file (with `prefix` junk bytes before it) and attach it to a
 * loop device. With `offset` != 0 the device starts at that file offset (LOOP_SET_STATUS64
 * after LOOP_SET_FD). Returns 0 or -errno. */
static A_UNUSED i64 p_sq_setup(struct p_sq *s, const char *dir, const unsigned char *img, size_t len, size_t prefix,
                               u64 offset) {
    p_sq_paths(s, dir);
    s->dev = -1;
    i64 r = p_mkdir(dir);
    if (r < 0) return r;
    r = a_mkdir(s->mnt, 0755);
    if (r < 0) return r;
    s->ctl = p_ctl_open(s->ctlp);
    if (s->ctl < 0) return s->ctl;
    i64 b = p_image(s->imgp, img, len, prefix);
    if (b < 0) return b;
    s->dev = p_attach(s->ctl, b, s->node, &s->num, p_sq_cfg);
    a_close(b);
    if (s->dev < 0) return s->dev;
    if (offset) {
        struct p_loop_info64 info;
        memset(&info, 0, sizeof info);
        info.lo_offset = offset;
        r = p_ioctl(s->dev, LOOP_SET_STATUS64, (i64)&info);
        if (r < 0) return r;
    }
    return 0;
}

/* Like p_sq_setup, but the file holds exactly the `len` bytes of `data` (no padding). */
static A_UNUSED i64 p_sq_setup_raw(struct p_sq *s, const char *dir, const unsigned char *data, size_t len) {
    p_sq_paths(s, dir);
    s->dev = -1;
    i64 r = p_mkdir(dir);
    if (r < 0) return r;
    r = a_mkdir(s->mnt, 0755);
    if (r < 0) return r;
    s->ctl = p_ctl_open(s->ctlp);
    if (s->ctl < 0) return s->ctl;
    i64 b = p_file(s->imgp, data, len);
    if (b < 0) return b;
    s->dev = p_attach(s->ctl, b, s->node, &s->num, 0);
    a_close(b);
    return s->dev < 0 ? s->dev : 0;
}

static A_UNUSED i64 p_sq_mount(struct p_sq *s) { return p_mount_squash(s->node, s->mnt, 0); }

/* Unmount (if mounted), detach and remove everything. */
static A_UNUSED void p_sq_teardown(struct p_sq *s) {
    while (p_umount(s->mnt) == 0) {
    }
    syscall2(SYS_umount2, s->mnt, MNT_DETACH);
    if (s->dev >= 0) p_detach(s->dev);
    if (s->ctl >= 0) a_close(s->ctl);
    a_unlink(s->ctlp);
    a_unlink(s->imgp);
    a_unlink(s->node);
    a_rmdir(s->mnt);
    a_rmdir(s->dir);
}

/* The path of `name` below the mount point, in `out`. */
static A_UNUSED const char *p_sq_path(struct p_sq *s, char *out, const char *name) {
    str_cat2(out, s->mnt, name);
    return out;
}

/*
 * Check the contents of the "small" image tree mounted at `mnt`: hello.txt (15 bytes),
 * d/inner.txt (6 bytes), the symlink l -> hello.txt and pattern.txt (9000 bytes, byte i is
 * "0123456789abcdef\n"[i % 17]), all owned by root with times 0. Each check is named
 * "<tag>:<what>".
 */
static A_UNUSED void p_c(int print, int *bad, const char *name, int ok, i64 got) {
    if (!ok) (*bad)++;
    if (print) check(name, ok, got);
}

static A_UNUSED int p_small_run(const char *tag, const char *mnt, int print) {
    int bad = 0;
    char name[96], path[160], buf[9100];
    struct a_stat st;
    str_cpy(str_cpy(name, tag), ":hello_stat");
    str_cat2(path, mnt, "/hello.txt");
    i64 r = syscall2(SYS_stat, path, &st);
    p_c(print, &bad, name, r == 0 && st.st_mode == (S_IFREG | 0644) && st.st_size == 15 && st.st_uid == 0 && st.st_gid == 0 &&
                    st.st_mtime_sec == 0, r);
    str_cpy(str_cpy(name, tag), ":hello_data");
    memset(buf, 0, sizeof buf);
    i64 fd = a_open(path, O_RDONLY, 0);
    i64 n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
    p_c(print, &bad, name, n == 15 && mem_eq(buf, "hello squashfs\n", 15), n);
    if (fd >= 0) a_close(fd);
    str_cpy(str_cpy(name, tag), ":inner_data");
    str_cat2(path, mnt, "/d/inner.txt");
    memset(buf, 0, sizeof buf);
    fd = a_open(path, O_RDONLY, 0);
    n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
    p_c(print, &bad, name, n == 6 && mem_eq(buf, "inner\n", 6), n);
    if (fd >= 0) a_close(fd);
    str_cpy(str_cpy(name, tag), ":symlink");
    str_cat2(path, mnt, "/l");
    memset(buf, 0, sizeof buf);
    n = syscall3(SYS_readlink, path, buf, sizeof buf);
    p_c(print, &bad, name, n == 9 && mem_eq(buf, "hello.txt", 9), n);
    str_cpy(str_cpy(name, tag), ":pattern_data");
    str_cat2(path, mnt, "/pattern.txt");
    memset(buf, 0, sizeof buf);
    fd = a_open(path, O_RDONLY, 0);
    n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
    int ok = n == 9000;
    static const char pat[] = "0123456789abcdef\n";
    for (int i = 0; ok && i < 9000; i++)
        if (buf[i] != pat[i % 17]) ok = 0;
    p_c(print, &bad, name, ok, n);
    str_cpy(str_cpy(name, tag), ":pattern_straddle");
    memset(buf, 0, sizeof buf);
    n = fd < 0 ? fd : syscall4(SYS_pread64, fd, buf, 200, 4000);
    ok = n == 200;
    for (int i = 0; ok && i < 200; i++)
        if (buf[i] != pat[(4000 + i) % 17]) ok = 0;
    p_c(print, &bad, name, ok, n);
    if (fd >= 0) a_close(fd);
    str_cpy(str_cpy(name, tag), ":pattern_size");
    syscall2(SYS_stat, path, &st);
    p_c(print, &bad, name, st.st_size == 9000, st.st_size);
    str_cpy(str_cpy(name, tag), ":statfs");
    struct a_statfs fs;
    r = syscall2(SYS_statfs, mnt, &fs);
    p_c(print, &bad, name, r == 0 && fs.f_type == 0x73717368 && (fs.f_files == 6), r);
    return bad;
}

static A_UNUSED void p_small_check(const char *tag, const char *mnt) { p_small_run(tag, mnt, 1); }

/* 1 when everything in the tree is as it should be (nothing is printed). */
static A_UNUSED int p_small_ok(const char *mnt) { return p_small_run("", mnt, 0) == 0; }

/* Set up `img` in directory `dir`, mount it and run p_small_check; leaves it unmounted. */
static A_UNUSED i64 p_small_case(struct p_sq *s, const char *tag, const char *dir, const unsigned char *img, size_t len) {
    char name[96];
    i64 r = p_sq_setup(s, dir, img, len, 0, 0);
    str_cpy(str_cpy(name, tag), ":setup");
    check(name, r == 0, r);
    if (r < 0) {
        p_sq_teardown(s);
        return r;
    }
    r = p_sq_mount(s);
    str_cpy(str_cpy(name, tag), ":mount");
    check(name, r == 0, r);
    if (r == 0) p_small_check(tag, s->mnt);
    p_sq_teardown(s);
    return r;
}

/* Remove every device this test added with p_add/p_attach (those still in use stay: EBUSY). */
static A_UNUSED void p_cleanup(void) {
    if (!p_added_n) return;
    const char *path = "/tmp/pk_cleanup_ctl";
    i64 c = p_ctl_open(path);
    for (int i = 0; c >= 0 && i < p_added_n; i++) p_ioctl(c, LOOP_CTL_REMOVE, p_added[i]);
    a_close(c);
    a_unlink(path);
    p_added_n = 0;
}

static A_UNUSED int p_finish(void) {
    p_cleanup();
    return finish();
}
#define finish p_finish

#endif
