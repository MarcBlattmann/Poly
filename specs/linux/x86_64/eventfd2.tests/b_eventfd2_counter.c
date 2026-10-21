/* eventfd2 counter: initval sets the counter; write adds an 8-byte value;
 * read returns the counter and resets it to 0 (EFD_SEMAPHORE: returns 1
 * and decrements); reading a zero counter in nonblocking mode is EAGAIN;
 * buffers smaller than 8 bytes are EINVAL (larger ones read 8); writing
 * 0xffffffffffffffff is EINVAL; a write that would pass
 * 0xfffffffffffffffe is EAGAIN when nonblocking. Flags: EFD_CLOEXEC,
 * EFD_NONBLOCK, EFD_SEMAPHORE; others EINVAL. A read into a bad buffer
 * is EFAULT and the value is lost (the counter was already reset); lseek
 * returns 0. */
#include "poly_ipc.h"

int test_main(void) {
    i64 fd = syscall2(SYS_eventfd2, 5, EFD_NONBLOCK);
    check("returns_lowest_fd", fd == 3, fd);
    check("status_rdwr_nonblock", syscall2(SYS_fcntl, fd, F_GETFL) == (O_RDWR | O_NONBLOCK), 0);
    u64 v = 0;
    i64 r = read((int)fd, &v, 8);
    check("read_returns_8", r == 8, r);
    check("initval_read", v == 5, (i64)v);
    r = read((int)fd, &v, 8);
    check("read_resets_to_zero_eagain", r == -EAGAIN, r);

    v = 3;
    write((int)fd, &v, 8);
    v = 4;
    r = write((int)fd, &v, 8);
    check("write_returns_8", r == 8, r);
    u64 big[2] = { 0, 0x5555 };
    r = read((int)fd, big, 16);
    check("larger_buffer_reads_8", r == 8 && big[0] == 7 && big[1] == 0x5555, r);

    r = read((int)fd, &v, 4);
    check("small_read_einval", r == -EINVAL, r);
    r = write((int)fd, &v, 4);
    check("small_write_einval", r == -EINVAL, r);
    v = ~0UL;
    r = write((int)fd, &v, 8);
    check("write_max_einval", r == -EINVAL, r);
    v = 0xfffffffffffffffeUL;
    r = write((int)fd, &v, 8);
    check("write_up_to_limit", r == 8, r);
    v = 1;
    r = write((int)fd, &v, 8);
    check("overflow_eagain", r == -EAGAIN, r);
    v = 0;
    r = write((int)fd, &v, 8);
    check("write_zero_ok_when_full", r == 8, r);
    read((int)fd, &v, 8);
    check("full_counter_read", v == 0xfffffffffffffffeUL, (i64)v);
    close((int)fd);

    /* Semaphore mode. */
    fd = syscall2(SYS_eventfd2, 2, EFD_SEMAPHORE | EFD_NONBLOCK | EFD_CLOEXEC);
    check("semaphore_cloexec", syscall2(SYS_fcntl, fd, F_GETFD) == FD_CLOEXEC, 0);
    read((int)fd, &v, 8);
    check("semaphore_reads_one", v == 1, (i64)v);
    read((int)fd, &v, 8);
    check("semaphore_second_one", v == 1, (i64)v);
    r = read((int)fd, &v, 8);
    check("semaphore_empty_eagain", r == -EAGAIN, r);
    close((int)fd);

    r = syscall2(SYS_eventfd2, 0, 0x10);
    check("unknown_flag_einval", r == -EINVAL, r);
    r = syscall2(SYS_eventfd2, 0, O_RDWR);
    check("o_rdwr_flag_einval", r == -EINVAL, r);
    fd = syscall2(SYS_eventfd2, 0, 0);
    check("flags_zero_blocking", syscall2(SYS_fcntl, fd, F_GETFL) == O_RDWR, 0);
    v = 1;
    write((int)fd, &v, 8);
    r = read((int)fd, (void *)8, 8);
    check("bad_buffer_efault", r == -EFAULT, r);
    struct k_pollfd pf = { (int)fd, POLLIN, 0 };
    r = syscall3(SYS_poll, &pf, 1, 0);
    check("failed_read_counter_already_reset", r == 0, r);
    r = syscall3(SYS_lseek, fd, 0, 0);
    check("lseek_returns_zero", r == 0, r);
    return finish();
}
