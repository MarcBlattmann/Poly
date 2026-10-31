/* splice(2) reads *off_in / *off_out before the transfer and writes the
 * advanced value back after it. If the offset can be read but not
 * written (read-only memory) the call fails with EFAULT, but the bytes
 * HAVE moved (into the pipe, or out of it into the file) and the value
 * in memory is unchanged. The file offset is not used or changed.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275
#define SYS_pread64 17
/* linux/fs.h */
#define SEEK_CUR 1

#define F "/tmp/c_splice_offset_writeback"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }
static int avail(int fd) {
    int n = -1;
    syscall3(SYS_ioctl, fd, FIONREAD, &n);
    return n;
}

int test_main(void) {
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    expect("fill", write(f, "0123456789", 10), 10);
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    i64 page = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", page > 0, page);
    i64 *ro = (i64 *)page;
    ro[0] = 2;
    ro[1] = 6;
    ro[2] = 100;
    expect("mprotect", syscall3(SYS_mprotect, page, 4096, PROT_READ), 0);
    char buf[32];

    expect("off_in_readonly_efault", splice(f, &ro[0], p[1], 0, 3, 0), -EFAULT);
    expect("off_in_value_kept", ro[0], 2);
    expect("bytes_moved_in", avail(p[0]), 3);
    expect("fpos_untouched", syscall3(SYS_lseek, f, 0, SEEK_CUR), 10);
    expect("read", read(p[0], buf, 32), 3);
    check("data_from_off_2", mem_eq(buf, "234", 3), 0);

    expect("put", write(p[1], "XYZ", 3), 3);
    expect("off_out_readonly_efault", splice(p[0], 0, f, &ro[1], 3, 0), -EFAULT);
    expect("off_out_value_kept", ro[1], 6);
    expect("pipe_drained", avail(p[0]), 0);
    expect("pread", syscall4(SYS_pread64, f, buf, 32, 0), 10);
    check("written_at_6", mem_eq(buf, "012345XYZ9", 10), 0);
    expect("fpos_untouched_2", syscall3(SYS_lseek, f, 0, SEEK_CUR), 10);

    /* A read-only offset past the end, nothing to move. */
    expect("past_eof_readonly", splice(f, &ro[2], p[1], 0, 3, 0), -EFAULT);
    expect("past_eof_value_kept", ro[2], 100);
    expect("past_eof_nothing_moved", avail(p[0]), 0);

    close(p[0]);
    close(p[1]);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
