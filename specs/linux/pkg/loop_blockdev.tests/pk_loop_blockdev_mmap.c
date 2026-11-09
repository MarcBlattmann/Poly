/* The loop device as a mappable, direct-I/O capable block device. A read-only shared mapping
 * of the device shows the file's bytes (after BLKFLSBUF); a writable shared mapping writes
 * them (msync, then fsync, then the file has them); mapping past the end of the device is
 * allowed but touching a page wholly beyond it kills with SIGBUS (not tested); O_DIRECT
 * reads of whole, aligned sectors work and return the file's bytes, an O_DIRECT read whose
 * offset or length is not a multiple of 512 is EINVAL; two descriptors with O_DIRECT and
 * without agree on the contents.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lb_mmap"

static char aligned[8192] __attribute__((aligned(4096)));

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[8192];
    for (int i = 0; i < 8192; i++) data[i] = (char)('A' + (i / 512) % 26);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 num = -1;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();

    /* mmap */
    i64 m = syscall6(SYS_mmap, 0, 8192, PROT_READ, MAP_SHARED, dev, 0);
    check("mmap_read", m > 0 || m < -4096, m);
    if (m > 0) {
        const char *p = (const char *)m;
        check("mapped_content", mem_eq(p, data, 8192), 0);
        expect("munmap", syscall2(SYS_munmap, m, 8192), 0);
    }
    i64 w = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, dev, 4096);
    check("mmap_write", w > 0 || w < -4096, w);
    if (w > 0) {
        char *p = (char *)w;
        p[10] = '#';
        p[11] = '!';
        expect("msync", syscall3(SYS_msync, w, 4096, MS_SYNC), 0);
        expect("munmap_w", syscall2(SYS_munmap, w, 4096), 0);
        expect("fsync", syscall1(SYS_fsync, dev), 0);
        char got[2] = {0, 0};
        expect("file_read", syscall4(SYS_pread64, b, got, 2, 4096 + 10), 2);
        check("file_has_marks", got[0] == '#' && got[1] == '!', got[0]);
    }
    /* a mapping opened read-only cannot be mapped writable */
    i64 rd = a_open(D "/l", O_RDONLY, 0);
    i64 bad = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, rd, 0);
    expect("rdonly_fd_shared_write_mmap_eacces", bad, -EACCES);
    a_close(rd);

    /* O_DIRECT */
    expect("flush", p_ioctl(dev, BLKFLSBUF, 0), 0);
    i64 dd = a_open(D "/l", O_RDWR | O_DIRECT, 0);
    check("open_direct", dd >= 0, dd);
    if (dd >= 0) {
        memset(aligned, 0, sizeof aligned);
        expect("direct_read_1024", syscall4(SYS_pread64, dd, aligned, 1024, 0), 1024);
        check("direct_content", mem_eq(aligned, data, 1024), 0);
        expect("direct_read_at_512", syscall4(SYS_pread64, dd, aligned, 512, 512), 512);
        check("direct_content_512", mem_eq(aligned, data + 512, 512), 0);
        expect("direct_read_unaligned_offset", syscall4(SYS_pread64, dd, aligned, 512, 100), -EINVAL);
        expect("direct_read_unaligned_length", syscall4(SYS_pread64, dd, aligned, 100, 0), -EINVAL);
        expect("direct_read_at_end", syscall4(SYS_pread64, dd, aligned, 512, 8192), 0);
        memset(aligned, 'D', 512);
        expect("direct_write_512", syscall4(SYS_pwrite64, dd, aligned, 512, 1024), 512);
        char got[4] = {0};
        expect("file_sees_direct_write", syscall4(SYS_pread64, b, got, 4, 1024), 4);
        check("direct_write_content", mem_eq(got, "DDDD", 4), 0);
        char c = 0;
        expect("buffered_fd_sees_direct_write", syscall4(SYS_pread64, dev, &c, 1, 1030), 1);
        expect("buffered_value", c, 'D');
        a_close(dd);
    }
    p_detach(dev);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
