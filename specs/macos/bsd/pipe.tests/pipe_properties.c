/* pipe, BSD 42: fstat shows a FIFO (mode S_IFIFO|0660, size 0); with
 * O_NONBLOCK an empty read fails with EAGAIN (35) and a write that does not
 * fit is short; FIONREAD says how many bytes are waiting. The capacity (how
 * much a nonblocking pipe takes before it is full) is a value. */
#include "poly_mtest.h"

static char big[100000];

int test_main(void) {
    sysret p = bsd0(SYS_pipe);
    struct poly_stat64 st;
    check_ok("fstat of the read end", bsd2(SYS_fstat64, p.ret, &st));
    check("a FIFO with mode 0660", st.st_mode == (S_IFIFO | 0660), st.st_mode);
    check("size 0", st.st_size == 0, st.st_size);
    bsd3(SYS_fcntl, p.ret, F_SETFL, O_NONBLOCK);
    bsd3(SYS_fcntl, p.ret2, F_SETFL, O_NONBLOCK);
    char b[8];
    check_err("an empty nonblocking read: EAGAIN", bsd3(SYS_read, p.ret, b, 8), 35);
    bsd3(SYS_write, p.ret2, "abc", 3);
    i32 n = -1;
    bsd3(SYS_ioctl, p.ret, FIONREAD, &n);
    check("FIONREAD: 3 bytes waiting", n == 3, n);
    sysret w = bsd3(SYS_write, p.ret2, big, sizeof big);
    check("a write larger than the pipe is short", w.err == 0 && w.ret > 0 && w.ret < (i64)sizeof big, w.ret);
    value("pipe capacity (bytes taken by the second write, after 3 bytes)", w.ret);
    check_err("a full pipe: EAGAIN", bsd3(SYS_write, p.ret2, "x", 1), 35);
    return finish();
}
