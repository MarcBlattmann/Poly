/* BLKROSET (0x125D) / BLKROGET (0x125E) on a loop device bound to a writable file: setting 1
 * makes writes fail with EPERM and BLKROGET read 1; setting 0 makes it writable again. A
 * descriptor opened O_RDONLY cannot write (EBADF); a read-only descriptor still reads.
 * A device whose file was opened read-only stays read-only whatever BLKROSET says (see
 * the loop attach tests); ftruncate on a block device is EINVAL; a block device cannot be
 * opened for writing as a directory, and O_DIRECTORY on it is ENOTDIR.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lb_ro"

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[4096];
    memset(data, 'r', sizeof data);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 num = -1;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();

    int ro = 9, one = 1, zero = 0;
    expect("blkroget_0", p_ioctl(dev, BLKROGET, (i64)&ro), 0);
    expect("initially_writable", ro, 0);
    expect("write_ok", syscall4(SYS_pwrite64, dev, "w", 1, 0), 1);
    expect("blkroset_1", p_ioctl(dev, BLKROSET, (i64)&one), 0);
    expect("blkroget_1", p_ioctl(dev, BLKROGET, (i64)&ro), 0);
    expect("now_read_only", ro, 1);
    expect("write_eperm", syscall4(SYS_pwrite64, dev, "w", 1, 0), -EPERM);
    expect("sequential_write_eperm", a_write(dev, "w", 1), -EPERM);
    char c = 0;
    expect("read_still_works", syscall4(SYS_pread64, dev, &c, 1, 0), 1);
    expect("read_back_w", c, 'w');
    i64 second = a_open(D "/l", O_RDWR, 0);
    expect("flag_is_per_device_second_write_eperm", syscall4(SYS_pwrite64, second, "w", 1, 0), -EPERM);
    a_close(second);
    expect("blkroset_0", p_ioctl(dev, BLKROSET, (i64)&zero), 0);
    expect("blkroget_back_0", p_ioctl(dev, BLKROGET, (i64)&ro), 0);
    expect("writable_again", ro, 0);
    expect("write_ok_again", syscall4(SYS_pwrite64, dev, "v", 1, 0), 1);

    /* a read-only descriptor */
    i64 rd = a_open(D "/l", O_RDONLY, 0);
    check("open_rdonly", rd >= 0, rd);
    expect("rdonly_write_ebadf", syscall4(SYS_pwrite64, rd, "x", 1, 0), -EBADF);
    expect("rdonly_read_ok", syscall4(SYS_pread64, rd, &c, 1, 0), 1);
    expect("rdonly_sees_v", c, 'v');
    expect("rdonly_blkroget", p_ioctl(rd, BLKROGET, (i64)&ro), 0);
    a_close(rd);

    /* odd requests */
    expect("ftruncate_einval", syscall2(SYS_ftruncate, dev, 0), -EINVAL);
    expect("open_o_directory_enotdir", a_open(D "/l", O_RDONLY | O_DIRECTORY, 0), -ENOTDIR);
    expect("fsync_ok", syscall1(SYS_fsync, dev), 0);
    expect("fdatasync_ok", syscall1(SYS_fdatasync, dev), 0);

    p_detach(dev);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
