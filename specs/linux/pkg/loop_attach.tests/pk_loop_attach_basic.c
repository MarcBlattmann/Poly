/* LOOP_SET_FD (0x4C00) binds an open regular file to a loop device: the device's size is the
 * file's size rounded down to 512-byte sectors; LOOP_GET_STATUS64 then reports the device
 * number, no flags, offset 0, no size limit and an empty file name; writes to the device
 * reach the file after fsync (or BLKFLSBUF), and the device shows what is in the file
 * (after BLKFLSBUF). A second LOOP_SET_FD is EBUSY, an empty file can be bound (size 0).
 * LOOP_CLR_FD (0x4C01) unbinds: LOOP_GET_STATUS64 is then ENXIO, a second LOOP_CLR_FD is
 * ENXIO, LOOP_SET_FD on the same open descriptor is still EBUSY (open it again).
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_la_basic"

static u64 devsize(i64 d) {
    u64 s = 77;
    if (p_ioctl(d, BLKGETSIZE64, (i64)&s) < 0) return (u64)-1;
    return s;
}

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[8192];
    memset(data, 'A', sizeof data);
    i64 b = p_file(D "/b", data, sizeof data);
    check("backing", b >= 0, b);
    i64 num = -1;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();

    struct p_loop_info64 info;
    memset(&info, 0xaa, sizeof info);
    expect("get_status64", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("status_number", info.lo_number, num);
    expect("status_flags_0", info.lo_flags, 0);
    expect("status_offset_0", info.lo_offset, 0);
    expect("status_sizelimit_0", info.lo_sizelimit, 0);
    expect("status_encrypt_type_0", info.lo_encrypt_type, 0);
    expect("status_name_empty", info.lo_file_name[0], 0);
    expect("size_8192", devsize(dev), 8192);

    expect("set_fd_again_ebusy", p_ioctl(dev, LOOP_SET_FD, b), -EBUSY);
    expect("set_fd_self_ebusy", p_ioctl(dev, LOOP_SET_FD, dev), -EBUSY);

    /* device -> file */
    expect("write_dev", syscall4(SYS_pwrite64, dev, "hello", 5, 100), 5);
    expect("fsync_dev", syscall1(SYS_fsync, dev), 0);
    char got[8];
    memset(got, 0, sizeof got);
    expect("read_file", syscall4(SYS_pread64, b, got, 5, 100), 5);
    check("file_has_hello", mem_eq(got, "hello", 5), 0);
    /* file -> device */
    expect("write_file", syscall4(SYS_pwrite64, b, "world", 5, 4096), 5);
    expect("flush_dev_cache", p_ioctl(dev, BLKFLSBUF, 0), 0);
    memset(got, 0, sizeof got);
    expect("read_dev", syscall4(SYS_pread64, dev, got, 5, 4096), 5);
    check("dev_has_world", mem_eq(got, "world", 5), 0);
    /* BLKFLSBUF also writes the device's cache to the file */
    expect("write_dev_2", syscall4(SYS_pwrite64, dev, "again", 5, 300), 5);
    expect("flush_dev_cache_2", p_ioctl(dev, BLKFLSBUF, 0), 0);
    memset(got, 0, sizeof got);
    syscall4(SYS_pread64, b, got, 5, 300);
    check("file_has_again", mem_eq(got, "again", 5), 0);

    expect("clr_fd", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
    expect("status_after_clr_enxio", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);
    expect("clr_fd_again_enxio", p_ioctl(dev, LOOP_CLR_FD, 0), -ENXIO);
    expect("set_fd_same_descriptor_ebusy", p_ioctl(dev, LOOP_SET_FD, b), -EBUSY);
    a_close(dev);

    /* an empty file */
    i64 e = p_file(D "/e", "", 0);
    dev = p_attach(ctl, e, D "/l", &num, 0);
    check("set_fd_empty_file", dev >= 0, dev);
    if (dev >= 0) {
        expect("empty_size_0", devsize(dev), 0);
        char c;
        expect("empty_read_0", a_read(dev, &c, 1), 0);
        expect("empty_write_enospc", a_write(dev, "x", 1), -ENOSPC);
        p_detach(dev);
    }
    /* the file can be bound again after the device was released */
    dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd_after_release", dev >= 0, dev);
    if (dev >= 0) {
        expect("size_again_8192", devsize(dev), 8192);
        p_detach(dev);
    }
    a_close(e);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/e");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
