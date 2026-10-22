/* mknod: device nodes as root. st_rdev reports the dev argument's low 32
 * bits (major in bits 8-19, minor in bits 0-7 and 20-31, as makedev(3)
 * builds it); statx splits it into major and minor. A device node keeps
 * the mode bits like any other node.
 * poly-diff: needs=root (only a privileged caller may make device nodes)
 */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_device_root"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

static i64 rdev(const char *p) {
    struct a_stat st;
    return a_lstat_path(p, &st) == 0 ? (i64)st.st_rdev : -1;
}

int test_main(void) {
    struct a_stat st;
    struct a_statx sx;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("chr_1_3", mk(D "/null", S_IFCHR | 0666, 0x0103), 0);
    expect("chr_1_3_stat", a_lstat_path(D "/null", &st), 0);
    expect("chr_1_3_mode", st.st_mode, S_IFCHR | 0644);
    expect("chr_1_3_rdev", (i64)st.st_rdev, 0x0103);
    expect("chr_1_3_size", st.st_size, 0);
    expect("chr_1_3_uid", st.st_uid, 0);

    expect("blk_8_1", mk(D "/sda1", S_IFBLK | 0660, 0x0801), 0);
    expect("blk_8_1_mode", a_lstat_path(D "/sda1", &st) == 0 ? st.st_mode : 0, S_IFBLK | 0640);
    expect("blk_8_1_rdev", (i64)st.st_rdev, 0x0801);

    /* Major 0x123 (bits 8-19), minor 0x45678 (bits 0-7 and 20-31). */
    i64 big = 0x78 | (0x123 << 8) | ((i64)0x456 << 20);
    expect("chr_big", mk(D "/big", S_IFCHR | 0600, big), 0);
    expect("chr_big_rdev", rdev(D "/big"), big);
    expect("chr_big_statx", syscall5(SYS_statx, AT_FDCWD, D "/big", AT_SYMLINK_NOFOLLOW, STATX_BASIC_STATS, &sx), 0);
    expect("chr_big_statx_major", sx.stx_rdev_major, 0x123);
    expect("chr_big_statx_minor", sx.stx_rdev_minor, 0x45678);

    expect("chr_high_bits_dropped", mk(D "/hi", S_IFCHR | 0600, 0x500000103L), 0);
    expect("chr_high_bits_dropped_rdev", rdev(D "/hi"), 0x0103);
    expect("chr_null_statx", syscall5(SYS_statx, AT_FDCWD, D "/null", 0, STATX_BASIC_STATS, &sx), 0);
    expect("chr_null_statx_major", sx.stx_rdev_major, 1);
    expect("chr_null_statx_minor", sx.stx_rdev_minor, 3);

    expect("chr_setuid_kept", mk(D "/su", S_IFCHR | 04755, 0x0105), 0);
    expect("chr_setuid_kept_mode", a_lstat_path(D "/su", &st) == 0 ? st.st_mode : 0, S_IFCHR | 04755);

    expect("chr_again_eexist", mk(D "/null", S_IFCHR | 0666, 0x0103), -EEXIST);
    expect("blk_over_chr_eexist", mk(D "/null", S_IFBLK | 0666, 0x0801), -EEXIST);
    expect("chr_missing_parent_enoent", mk(D "/nope/c", S_IFCHR | 0666, 0x0103), -ENOENT);

    expect("cleanup_null", a_unlink(D "/null"), 0);
    expect("cleanup_sda1", a_unlink(D "/sda1"), 0);
    expect("cleanup_big", a_unlink(D "/big"), 0);
    expect("cleanup_hi", a_unlink(D "/hi"), 0);
    expect("cleanup_su", a_unlink(D "/su"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
