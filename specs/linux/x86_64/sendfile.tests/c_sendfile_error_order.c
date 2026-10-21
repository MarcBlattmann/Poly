/* Which error wins: an unreadable offset pointer is EFAULT before any
 * descriptor is looked at (also with count 0); a bad descriptor is EBADF
 * before a negative offset or a negative count is noticed.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_error_order_in"
#define G "/tmp/c_sendfile_error_order_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    i64 off;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    i64 *bad = (i64 *)8;
    expect("bad_pointer", sendfile(out, in, bad, 5), -EFAULT);
    expect("bad_pointer_count_0", sendfile(out, in, bad, 0), -EFAULT);
    expect("bad_pointer_closed_in", sendfile(out, 40, bad, 1), -EFAULT);
    expect("bad_pointer_closed_both", sendfile(41, 40, bad, 1), -EFAULT);
    expect("bad_pointer_negative_count", sendfile(out, in, bad, (u64)-1), -EFAULT);
    off = -1;
    expect("closed_before_negative_offset", sendfile(41, 40, &off, 1), -EBADF);
    expect("closed_in_before_negative_offset", sendfile(out, 40, &off, 1), -EBADF);
    expect("closed_before_negative_count", sendfile(41, 40, 0, (u64)-1), -EBADF);
    expect("closed_in_before_negative_count", sendfile(out, 40, 0, (u64)-1), -EBADF);
    i64 wo = a_open(F, O_WRONLY, 0);
    expect("mode_before_negative_count", sendfile(out, wo, 0, (u64)-1), -EBADF);
    expect("negative_offset_valid_fds", sendfile(out, in, &off, 1), -EINVAL);
    a_close(wo);
    a_close(in);
    a_close(out);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
