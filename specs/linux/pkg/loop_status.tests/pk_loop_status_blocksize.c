/* LOOP_SET_BLOCK_SIZE (0x4C09, argument = bytes) sets the device's logical block size: 512 is
 * the default, 512, 1024 and 4096 are accepted (BLKSSZGET reads it back), a value that is
 * not a power of two (1000) or is below 512 (256) is EINVAL and changes nothing; an unbound
 * device answers ENXIO. The device size does not change (it stays the file size rounded to
 * 512 bytes; data stays the same). LOOP_SET_DIRECT_IO (0x4C08) with 0 clears
 * LO_FLAGS_DIRECT_IO and succeeds; an unbound device answers ENXIO.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_ls_blocksize"

static int ssz(i64 d) {
    int v = -1;
    if (p_ioctl(d, BLKSSZGET, (i64)&v) < 0) return -2;
    return v;
}

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[12288];
    for (int i = 0; i < 12288; i++) data[i] = (char)('0' + (i / 512) % 10);
    i64 b = p_file(D "/b", data, sizeof data);

    i64 n = p_add(ctl);
    expect("mknod", p_node(D "/u", n), 0);
    i64 un = a_open(D "/u", O_RDWR, 0);
    expect("unbound_set_block_size_enxio", p_ioctl(un, LOOP_SET_BLOCK_SIZE, 4096), -ENXIO);
    expect("unbound_set_direct_io_enxio", p_ioctl(un, LOOP_SET_DIRECT_IO, 0), -ENXIO);
    a_close(un);
    expect("remove", p_ioctl(ctl, LOOP_CTL_REMOVE, n), 0);

    i64 num;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();
    expect("default_512", ssz(dev), 512);
    expect("set_4096", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 4096), 0);
    expect("now_4096", ssz(dev), 4096);
    u64 size = 0;
    p_ioctl(dev, BLKGETSIZE64, (i64)&size);
    expect("size_unchanged", size, 12288);
    char c = 0;
    expect("read_at_4608", syscall4(SYS_pread64, dev, &c, 1, 4608), 1);
    expect("data_unchanged", c, '9');
    expect("set_1024", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 1024), 0);
    expect("now_1024", ssz(dev), 1024);
    expect("set_256_einval", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 256), -EINVAL);
    expect("still_1024", ssz(dev), 1024);
    expect("set_1000_einval", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 1000), -EINVAL);
    expect("set_100_einval", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 100), -EINVAL);
    expect("set_3000_einval", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 3000), -EINVAL);
    expect("still_1024_again", ssz(dev), 1024);
    expect("set_512", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 512), 0);
    expect("now_512", ssz(dev), 512);

    struct p_loop_info64 info;
    expect("direct_io_off", p_ioctl(dev, LOOP_SET_DIRECT_IO, 0), 0);
    expect("get_status64", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("direct_io_flag_clear", info.lo_flags & LO_FLAGS_DIRECT_IO, 0);

    /* a size that is not a multiple of the new block size */
    expect("odd_file", syscall2(SYS_ftruncate, b, 12288 + 600), 0);
    expect("set_capacity", p_ioctl(dev, LOOP_SET_CAPACITY, 0), 0);
    size = 0;
    p_ioctl(dev, BLKGETSIZE64, (i64)&size);
    expect("size_12800", size, 12800);
    expect("set_4096_again", p_ioctl(dev, LOOP_SET_BLOCK_SIZE, 4096), 0);
    size = 0;
    p_ioctl(dev, BLKGETSIZE64, (i64)&size);
    expect("size_still_12800", size, 12800);

    p_detach(dev);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/u");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
