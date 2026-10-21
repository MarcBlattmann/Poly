/* An eventfd made blocking (eventfd) can be switched to nonblocking with
 * fcntl(F_SETFL, O_NONBLOCK): a read of a zero counter is then EAGAIN; a
 * write that would overflow is EAGAIN; readv and writev with one 8-byte
 * element work like read and write; FIONREAD is not supported (ENOTTY). */
#include "poly_ipc.h"

#define SYS_readv 19
#define SYS_writev 20

int test_main(void) {
    int fd = (int)syscall1(SYS_eventfd, 0);
    i64 r = syscall3(SYS_fcntl, fd, F_SETFL, O_NONBLOCK);
    check("setfl_ok", r == 0, r);
    check("now_nonblocking", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | O_NONBLOCK), 0);
    u64 v;
    r = read(fd, &v, 8);
    check("zero_counter_eagain", r == -EAGAIN, r);
    v = 0xfffffffffffffffeUL;
    write(fd, &v, 8);
    v = 1;
    r = write(fd, &v, 8);
    check("overflow_eagain", r == -EAGAIN, r);
    read(fd, &v, 8);

    u64 w = 12;
    struct k_iovec iov = { &w, 8 };
    r = syscall3(SYS_writev, fd, &iov, 1);
    check("writev_one_element", r == 8, r);
    u64 got = 0;
    struct k_iovec riov = { &got, 8 };
    r = syscall3(SYS_readv, fd, &riov, 1);
    check("readv_one_element", r == 8 && got == 12, (i64)got);
    int n = 0;
    r = syscall3(SYS_ioctl, fd, FIONREAD, &n);
    check("fionread_enotty", r == -ENOTTY, r);
    return finish();
}
