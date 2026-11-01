/* The order of splice(2)'s checks: len 0 returns 0 before anything else
 * (bad descriptors, bad flags); then flags other than SPLICE_F_MOVE,
 * NONBLOCK, MORE, GIFT (0xf) are EINVAL; then each descriptor must be
 * open (EBADF); then an offset given for a pipe side is ESPIPE (the
 * pointer is not read); then the offsets are read (EFAULT); then
 * fd_in must be open for reading and fd_out for writing (EBADF); then
 * the kinds and values (neither side a pipe, a negative offset, a len
 * with bit 63 set when a file is involved: EINVAL). Only the low 32 bits
 * of the descriptors and of flags count.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_splice 275

#define F "/tmp/c_splice_error_order"

static i64 splice(i64 in, i64 *offin, i64 out, i64 *offout, u64 len, u64 flags) {
    return syscall6(SYS_splice, in, (i64)offin, out, (i64)offout, (i64)len, (i64)flags);
}
static void expect(const char *n, i64 got, i64 want) { check(n, got == want, got); }

int test_main(void) {
    i64 f = syscall4(SYS_openat, AT_FDCWD, F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("open", f >= 0, f);
    expect("fill", write(f, "0123456789", 10), 10);
    i64 fw = syscall4(SYS_openat, AT_FDCWD, F, O_WRONLY, 0);
    i64 fr = syscall4(SYS_openat, AT_FDCWD, F, O_RDONLY, 0);
    check("open_modes", fw >= 0 && fr >= 0, fw);
    int p[2];
    expect("pipe", syscall1(SYS_pipe, p), 0);
    i64 off = 0;

    expect("len0_bad_fds", splice(-1, 0, -1, 0, 0, 0), 0);
    expect("len0_bad_flags", splice(-1, 0, -1, 0, 0, 0x100), 0);
    expect("len0_bad_ptr", splice(f, (i64 *)8, p[1], 0, 0, 0), 0);
    expect("flags_before_ebadf", splice(-1, 0, -1, 0, 5, 0x10), -EINVAL);
    expect("flags_valid_fds", splice(f, 0, p[1], 0, 5, 0x100), -EINVAL);
    expect("ebadf_in", splice(-1, 0, p[1], 0, 5, 0), -EBADF);
    expect("ebadf_out", splice(f, 0, 77, 0, 5, 0), -EBADF);
    expect("ebadf_before_efault", splice(-1, (i64 *)8, p[1], 0, 5, 0), -EBADF);
    expect("ebadf_out_before_efault", splice(f, (i64 *)8, 77, 0, 5, 0), -EBADF);
    expect("ebadf_out_before_espipe", splice(p[0], &off, 77, 0, 5, 0), -EBADF);
    expect("espipe_before_efault", splice(f, (i64 *)8, p[1], &off, 5, 0), -ESPIPE);
    expect("espipe_in_before_efault_out", splice(p[0], &off, f, (i64 *)8, 5, 0), -ESPIPE);
    expect("espipe_before_mode", splice(p[1], &off, p[1], 0, 5, 0), -ESPIPE);
    expect("efault_off_in", splice(f, (i64 *)8, p[1], 0, 5, 0), -EFAULT);
    expect("efault_off_out", splice(p[0], 0, f, (i64 *)8, 5, 0), -EFAULT);
    expect("efault_before_mode", splice(fw, (i64 *)8, p[1], 0, 5, 0), -EFAULT);
    expect("efault_before_no_pipe", splice(f, (i64 *)8, f, 0, 5, 0), -EFAULT);
    expect("mode_in_wronly", splice(fw, 0, p[1], 0, 5, 0), -EBADF);
    expect("mode_out_rdonly", splice(p[0], 0, fr, 0, 5, 0), -EBADF);
    expect("mode_out_read_end", splice(f, 0, p[0], 0, 5, 0), -EBADF);
    off = -5;
    expect("mode_before_negative_off", splice(f, &off, p[0], 0, 5, 0), -EBADF);
    expect("negative_off_in", splice(f, &off, p[1], 0, 5, 0), -EINVAL);
    expect("no_pipe_einval", splice(f, 0, f, 0, 5, 0), -EINVAL);
    off = 0;
    expect("no_pipe_with_offsets_einval", splice(f, &off, fw, &off, 5, 0), -EINVAL);
    expect("len_bit63_file_in", splice(f, &off, p[1], 0, 1UL << 63, 0), -EINVAL);
    expect("len_minus1_file_in", splice(f, &off, p[1], 0, (u64)-1, 0), -EINVAL);
    expect("put", write(p[1], "abc", 3), 3);
    expect("len_bit63_file_out", splice(p[0], 0, f, &off, 1UL << 63, 0), -EINVAL);
    off = 0x7fffffffffffff00L;
    expect("off_plus_len_overflow", splice(f, &off, p[1], 0, 0x1000, 0), -EINVAL);

    /* High halves of the descriptor and flags registers are ignored. */
    char buf[16];
    expect("drain", read(p[0], buf, 16), 3);
    expect("rewind", syscall3(SYS_lseek, f, 0, 0), 0);
    expect("fd_high_bits", syscall6(SYS_splice, f | (1L << 32), 0, p[1] | (5L << 33), 0, 3, 0), 3);
    expect("flags_high_bits", syscall6(SYS_splice, f, 0, p[1], 0, 3, 1L << 32), 3);
    expect("read", read(p[0], buf, 16), 6);
    check("data", mem_eq(buf, "012345", 6), 0);

    close(p[0]);
    close(p[1]);
    close(fw);
    close(fr);
    close(f);
    expect("unlink", syscall1(SYS_unlink, F), 0);
    return finish();
}
