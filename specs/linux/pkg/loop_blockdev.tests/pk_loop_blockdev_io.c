/* Reads and writes at the end of a loop block device (size 1536 bytes: a 1700-byte file).
 * A read that starts before the end is cut short at the end (36 bytes at 1500), a read at or
 * past the end returns 0; a write that starts before the end is cut short (36 bytes), a write
 * at or past the end fails with ENOSPC. A write of a few bytes in the middle of a sector keeps
 * the rest of the sector. Written data reaches the file after fsync, BLKFLSBUF drops cached
 * data so later reads show the file again, and the file's tail beyond the device is never
 * touched.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lb_io"

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[2048], buf[2048];
    for (int i = 0; i < 2048; i++) data[i] = (char)('a' + i / 512);
    i64 b = p_file(D "/b", data, 1700);
    i64 num = -1;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();

    memset(buf, 0, sizeof buf);
    expect("read_all", syscall4(SYS_pread64, dev, buf, 2048, 0), 1536);
    check("read_all_content", mem_eq(buf, data, 1536), 0);
    memset(buf, 0, sizeof buf);
    expect("read_across_end", syscall4(SYS_pread64, dev, buf, 100, 1500), 36);
    check("read_across_end_content", mem_eq(buf, data + 1500, 36), 0);
    expect("read_at_end", syscall4(SYS_pread64, dev, buf, 100, 1536), 0);
    expect("read_past_end", syscall4(SYS_pread64, dev, buf, 100, 100000), 0);
    expect("read_zero_bytes", syscall4(SYS_pread64, dev, buf, 0, 10), 0);
    expect("seek_end", syscall3(SYS_lseek, dev, 0, SEEK_END), 1536);
    expect("sequential_read_at_end", a_read(dev, buf, 10), 0);
    expect("seek_back", syscall3(SYS_lseek, dev, 1530, SEEK_SET), 1530);
    expect("sequential_read_short", a_read(dev, buf, 100), 6);
    expect("then_eof", a_read(dev, buf, 100), 0);

    /* writes */
    expect("write_middle_of_sector", syscall4(SYS_pwrite64, dev, "XYZ", 3, 5), 3);
    memset(buf, 0, sizeof buf);
    expect("read_back", syscall4(SYS_pread64, dev, buf, 12, 0), 12);
    check("sector_kept_around_write", mem_eq(buf, "aaaaaXYZaaaa", 12), 0);
    expect("write_across_end_is_short", syscall4(SYS_pwrite64, dev, data, 100, 1500), 36);
    expect("write_last_byte", syscall4(SYS_pwrite64, dev, "L", 1, 1535), 1);
    expect("write_at_end_enospc", syscall4(SYS_pwrite64, dev, "x", 1, 1536), -ENOSPC);
    expect("write_past_end_enospc", syscall4(SYS_pwrite64, dev, "x", 1, 100000), -ENOSPC);
    expect("write_zero_bytes_at_end", syscall4(SYS_pwrite64, dev, "x", 0, 1536), 0);
    expect("seek_to_end", syscall3(SYS_lseek, dev, 0, SEEK_END), 1536);
    expect("sequential_write_at_end_enospc", a_write(dev, "x", 1), -ENOSPC);

    /* to the file */
    expect("fsync", syscall1(SYS_fsync, dev), 0);
    memset(buf, 0, sizeof buf);
    expect("read_file", syscall4(SYS_pread64, b, buf, 1700, 0), 1700);
    check("file_has_XYZ", mem_eq(buf + 5, "XYZ", 3), 0);
    expect("file_last_device_byte", buf[1535], 'L');
    check("file_tail_untouched", mem_eq(buf + 1536, data + 1536, 164), 0);
    check("file_rest_untouched", mem_eq(buf + 8, data + 8, 1500 - 8), 0);

    /* from the file: change it behind the device's back and flush the cache */
    expect("write_file", syscall4(SYS_pwrite64, b, "NEW", 3, 700), 3);
    expect("blkflsbuf", p_ioctl(dev, BLKFLSBUF, 0), 0);
    memset(buf, 0, sizeof buf);
    expect("read_new", syscall4(SYS_pread64, dev, buf, 3, 700), 3);
    check("device_shows_file_change", mem_eq(buf, "NEW", 3), 0);

    /* writes through a second descriptor are seen by the first */
    i64 d2 = a_open(D "/l", O_RDWR, 0);
    expect("write_second_fd", syscall4(SYS_pwrite64, d2, "two", 3, 1000), 3);
    memset(buf, 0, sizeof buf);
    expect("read_first_fd", syscall4(SYS_pread64, dev, buf, 3, 1000), 3);
    check("first_fd_sees_write", mem_eq(buf, "two", 3), 0);
    a_close(d2);

    p_detach(dev);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
