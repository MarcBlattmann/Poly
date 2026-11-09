/* LOOP_CONFIGURE (0x4C0A) binds a file and sets the status in one call. struct loop_config
 * is {u32 fd; u32 block_size; struct loop_info64 info; u64 reserved[8]}. The flags in
 * info (LO_FLAGS_READ_ONLY 1, LO_FLAGS_AUTOCLEAR 4) take effect at once, as do info's
 * file name, offset and size limit; block_size 0 keeps 512, 512/1024/4096 set the logical
 * block size (BLKSSZGET), anything else (256, 1000) is EINVAL and binds nothing.
 * A bound device answers LOOP_CONFIGURE with EBUSY, a bad descriptor is EBADF, a directory
 * EINVAL, a NULL argument EFAULT. With LO_FLAGS_AUTOCLEAR the device is unbound by the last
 * close.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_la_configure"

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
    for (int i = 0; i < 16384; i++) data[i] = (char)('a' + (i / 512) % 26);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 n = p_add(ctl);
    check("add_device", n >= 0, n);
    expect("mknod", p_node(D "/l", n), 0);
    i64 dev = a_open(D "/l", O_RDWR, 0);
    check("open_device", dev >= 0, dev);

    struct p_loop_config cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.fd = (u32)b;
    struct p_loop_info64 info;
    /* things that fail leave the device unbound */
    expect("null_argument_efault", p_ioctl(dev, LOOP_CONFIGURE, 0), -EFAULT);
    cfg.block_size = 1000;
    expect("block_size_1000_einval", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), -EINVAL);
    cfg.block_size = 256;
    expect("block_size_256_einval", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), -EINVAL);
    cfg.block_size = 0;
    cfg.fd = 99;
    expect("bad_fd_ebadf", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), -EBADF);
    i64 dirfd = a_open(D, O_RDONLY | O_DIRECTORY, 0);
    cfg.fd = (u32)dirfd;
    expect("directory_einval", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), -EINVAL);
    expect("still_unbound", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);

    /* plain configure: like LOOP_SET_FD, with a file name */
    cfg.fd = (u32)b;
    str_cpy((char *)cfg.info.lo_file_name, "named");
    expect("configure", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), 0);
    expect("configure_again_ebusy", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), -EBUSY);
    expect("set_fd_after_configure_ebusy", p_ioctl(dev, LOOP_SET_FD, b), -EBUSY);
    expect("get_status64", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    check("file_name_kept", str_eq((char *)info.lo_file_name, "named"), 0);
    expect("flags_0", info.lo_flags, 0);
    expect("size_16384", devsize(dev), 16384);
    int bs = 0;
    expect("blkssz", p_ioctl(dev, BLKSSZGET, (i64)&bs), 0);
    expect("block_size_512_default", bs, 512);
    expect("clr_fd", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
    a_close(dev);

    /* offset, size limit, read-only, autoclear and 4096-byte blocks, all at once */
    dev = a_open(D "/l", O_RDWR, 0);
    memset(&cfg, 0, sizeof cfg);
    cfg.fd = (u32)b;
    cfg.block_size = 4096;
    cfg.info.lo_offset = 4096;
    cfg.info.lo_sizelimit = 8192;
    cfg.info.lo_flags = LO_FLAGS_READ_ONLY | LO_FLAGS_AUTOCLEAR;
    expect("configure_all", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), 0);
    expect("get_status64_all", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("flags_ro_autoclear", info.lo_flags, LO_FLAGS_READ_ONLY | LO_FLAGS_AUTOCLEAR);
    expect("offset_4096", info.lo_offset, 4096);
    expect("sizelimit_8192", info.lo_sizelimit, 8192);
    expect("size_8192", devsize(dev), 8192);
    expect("blkssz_4096", p_ioctl(dev, BLKSSZGET, (i64)&bs), 0);
    expect("block_size_4096", bs, 4096);
    char c = 0;
    expect("read_first_byte", syscall4(SYS_pread64, dev, &c, 1, 0), 1);
    expect("first_byte_is_file_offset_4096", c, 'a' + 8);
    expect("write_eperm", syscall4(SYS_pwrite64, dev, "x", 1, 0), -EPERM);
    /* the last close unbinds an autoclear device */
    a_close(dev);
    dev = a_open(D "/l", O_RDWR, 0);
    expect("autoclear_unbound_after_close", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);

    /* offset beyond the end of the file: a device of size 0; block size 1024 */
    memset(&cfg, 0, sizeof cfg);
    cfg.fd = (u32)b;
    cfg.block_size = 1024;
    cfg.info.lo_offset = 20000;
    expect("configure_offset_past_end", p_ioctl(dev, LOOP_CONFIGURE, (i64)&cfg), 0);
    expect("size_0", devsize(dev), 0);
    expect("blkssz_1024", p_ioctl(dev, BLKSSZGET, (i64)&bs), 0);
    expect("block_size_1024", bs, 1024);
    expect("clr_fd_2", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
    a_close(dev);

    /* an autoclear device also goes when the descriptor that configured it is closed while
     * nobody else has it open: remove works afterwards */
    expect("remove", p_ioctl(ctl, LOOP_CTL_REMOVE, n), 0);
    a_close(dirfd);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
