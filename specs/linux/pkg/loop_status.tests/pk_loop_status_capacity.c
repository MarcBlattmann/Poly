/* LOOP_SET_CAPACITY (0x4C07, argument ignored) re-reads the size of the backing file after it
 * was grown or shrunk behind the device's back: the device keeps its old size until then.
 * The new size is the file size minus the offset (capped by the size limit), rounded down to
 * sectors; an unbound device answers ENXIO. The size is visible through BLKGETSIZE64,
 * BLKGETSIZE (in 512-byte sectors) and lseek(SEEK_END).
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_ls_capacity"

static u64 devsize(i64 d) {
    u64 s = 77;
    if (p_ioctl(d, BLKGETSIZE64, (i64)&s) < 0) return (u64)-1;
    return s;
}

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[16384];
    for (int i = 0; i < 16384; i++) data[i] = (char)('A' + (i / 512) % 26);
    i64 b = p_file(D "/b", data, 8192);
    i64 num;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();

    expect("size_8192", devsize(dev), 8192);
    expect("grow_file", syscall2(SYS_ftruncate, b, 32768), 0);
    expect("size_unchanged_until_asked", devsize(dev), 8192);
    expect("set_capacity_grow", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("size_32768", devsize(dev), 32768);
    u64 sectors = 0;
    expect("blkgetsize", p_ioctl(dev, BLKGETSIZE, (i64)&sectors), 0);
    expect("sectors_64", sectors, 64);
    expect("seek_end", syscall3(SYS_lseek, dev, 0, SEEK_END), 32768);
    char c = 1;
    expect("grown_part_reads_zero", syscall4(SYS_pread64, dev, &c, 1, 20000), 1);
    expect("zero_byte", c, 0);
    expect("write_into_grown_part", syscall4(SYS_pwrite64, dev, "Q", 1, 30000), 1);
    expect("fsync", syscall1(SYS_fsync, dev), 0);
    expect("file_has_it", syscall4(SYS_pread64, b, &c, 1, 30000), 1);
    expect("file_byte", c, 'Q');

    expect("shrink_file", syscall2(SYS_ftruncate, b, 8192), 0);
    expect("set_capacity_shrink", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("size_back_8192", devsize(dev), 8192);
    expect("read_past_end", syscall4(SYS_pread64, dev, &c, 1, 8192), 0);

    expect("odd_file_size", syscall2(SYS_ftruncate, b, 8200), 0);
    expect("set_capacity_odd", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("rounded_down_to_sector", devsize(dev), 8192);
    expect("odd_file_size_1000", syscall2(SYS_ftruncate, b, 8704 + 100), 0);
    expect("set_capacity_odd_2", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("size_8704", devsize(dev), 8704);
    expect("set_capacity_junk_argument", p_ioctl(dev, LOOP_SET_CAPACITY, 5), 0);

    /* with an offset and a limit */
    struct p_loop_info64 info;
    memset(&info, 0, sizeof info);
    info.lo_offset = 4096;
    info.lo_sizelimit = 6144;
    expect("set_offset_and_limit", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
    expect("size_limited", devsize(dev), 4608); /* file 8804 - 4096 = 4708 -> 4608 */
    expect("grow_again", syscall2(SYS_ftruncate, b, 65536), 0);
    expect("set_capacity_limited", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("size_is_limit", devsize(dev), 6144);
    expect("shrink_below_limit", syscall2(SYS_ftruncate, b, 5000), 0);
    expect("set_capacity_below", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    expect("size_904_rounded", devsize(dev), 512);

    p_detach(dev);
    /* an unbound device */
    i64 n = p_add(ctl);
    expect("mknod", p_node(D "/l2", n), 0);
    i64 d2 = a_open(D "/l2", O_RDWR, 0);
    expect("unbound_set_capacity_enxio", p_ioctl(d2, LOOP_SET_CAPACITY, 0), -ENXIO);
    a_close(d2);
    expect("remove", p_ioctl(ctl, LOOP_CTL_REMOVE, n), 0);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/l2");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
