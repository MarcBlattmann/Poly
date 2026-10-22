/* mknod: the file type in mode. Types other than regular, FIFO, socket,
 * character and block device fail with EINVAL; S_IFDIR fails with EPERM;
 * nothing is created. Only the low 16 bits of mode count. */
#include "a_sys.h"

#define SYS_mknod 133
#define D "/tmp/a_mknod_types"

static i64 mk(const char *p, i64 mode, i64 dev) { return syscall3(SYS_mknod, p, mode, dev); }

int test_main(void) {
    struct a_stat st;
    syscall1(SYS_umask, 022);
    expect("setup_mkdir", a_mkdir(D, 0755), 0);

    expect("symlink_type_einval", mk(D "/x", S_IFLNK | 0777, 0), -EINVAL);
    expect("type_0170000_einval", mk(D "/x", 0170000 | 0644, 0), -EINVAL);
    expect("type_0030000_einval", mk(D "/x", 0030000 | 0644, 0), -EINVAL);
    expect("type_0050000_einval", mk(D "/x", 0050000 | 0644, 0), -EINVAL);
    expect("type_0070000_einval", mk(D "/x", 0070000 | 0644, 0), -EINVAL);
    expect("type_0110000_einval", mk(D "/x", 0110000 | 0644, 0), -EINVAL);
    expect("type_0130000_einval", mk(D "/x", 0130000 | 0644, 0), -EINVAL);
    expect("type_0150000_einval", mk(D "/x", 0150000 | 0644, 0), -EINVAL);
    expect("type_0160000_einval", mk(D "/x", 0160000 | 0644, 0), -EINVAL);
    expect("mode_minus1_einval", mk(D "/x", -1, 0), -EINVAL);
    expect("mode_ffff_einval", mk(D "/x", 0xffff, 0), -EINVAL);
    expect("dir_type_eperm", mk(D "/x", S_IFDIR | 0755, 0), -EPERM);
    expect("dir_type_bare_eperm", mk(D "/x", S_IFDIR, 0), -EPERM);
    expect("nothing_created", a_lstat_path(D "/x", &st), -ENOENT);

    /* Bits above the low 16 do not count. */
    expect("high_bit_fifo", mk(D "/hf", 0x10000 | S_IFIFO | 0644, 0), 0);
    expect("high_bit_fifo_mode", a_lstat_path(D "/hf", &st) == 0 ? st.st_mode : 0, S_IFIFO | 0644);
    expect("high_bit_type0", mk(D "/hr", 0x7fff0000 | 0640, 0), 0);
    expect("high_bit_type0_mode", a_lstat_path(D "/hr", &st) == 0 ? st.st_mode : 0, S_IFREG | 0640);
    expect("high_bits_bad_type_einval", mk(D "/x", 0x1f000 | 0644, 0), -EINVAL);
    expect("high_bits_dir_eperm", mk(D "/x", 0x30000 | S_IFDIR | 0644, 0), -EPERM);

    /* Each allowed type an unprivileged caller may make. */
    expect("reg", mk(D "/reg", S_IFREG | 0600, 0), 0);
    expect("reg_type", a_lstat_path(D "/reg", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFREG);
    expect("fifo", mk(D "/fifo", S_IFIFO | 0600, 0), 0);
    expect("fifo_type", a_lstat_path(D "/fifo", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFIFO);
    expect("sock", mk(D "/sock", S_IFSOCK | 0600, 0), 0);
    expect("sock_type", a_lstat_path(D "/sock", &st) == 0 ? (i64)(st.st_mode & S_IFMT) : -1, S_IFSOCK);

    expect("cleanup_hf", a_unlink(D "/hf"), 0);
    expect("cleanup_hr", a_unlink(D "/hr"), 0);
    expect("cleanup_reg", a_unlink(D "/reg"), 0);
    expect("cleanup_fifo", a_unlink(D "/fifo"), 0);
    expect("cleanup_sock", a_unlink(D "/sock"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
