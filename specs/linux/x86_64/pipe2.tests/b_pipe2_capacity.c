/* Pipe capacity: a new pipe holds 65536 bytes (F_GETPIPE_SZ); a
 * nonblocking writer fills it and then gets EAGAIN; a write of at most
 * PIPE_BUF (4096) bytes is all-or-nothing, a bigger one is partial when
 * only some space is left; space is freed a page (4096 bytes) at a time,
 * so a partly read page takes no new bytes. F_SETPIPE_SZ rounds up to a power-of-two
 * number of pages and returns the new size; it cannot go below the bytes
 * already in the pipe (EBUSY); 0 gives one page. */
#include "poly_ipc.h"

static char big[70000];

int test_main(void) {
    int p[2];
    syscall2(SYS_pipe2, p, O_NONBLOCK);
    i64 r = syscall2(SYS_fcntl, p[0], F_GETPIPE_SZ);
    check("default_size_65536", r == 65536, r);
    check("write_end_same_size", syscall2(SYS_fcntl, p[1], F_GETPIPE_SZ) == 65536, 0);

    r = write(p[1], big, sizeof big);
    check("big_write_partial_fill", r == 65536, r);
    r = write(p[1], big, 1);
    check("full_pipe_eagain", r == -EAGAIN, r);
    int avail = 0;
    syscall3(SYS_ioctl, p[0], FIONREAD, &avail);
    check("fionread_full", avail == 65536, avail);

    /* Space comes back a page at a time: after reading 100 bytes of a full
     * pipe even a 50-byte write does not fit; once a whole page (4096
     * bytes) has been read, a 4096-byte write fits whole. */
    read(p[0], big, 100);
    r = write(p[1], big, 4096);
    check("pipe_buf_write_all_or_nothing", r == -EAGAIN, r);
    r = write(p[1], big, 50);
    check("partly_read_page_not_reused", r == -EAGAIN, r);
    read(p[0], big, 4096 - 100);
    r = write(p[1], big, 4096);
    check("freed_page_takes_pipe_buf_write", r == 4096, r);
    r = write(p[1], big, 1);
    check("full_again_eagain", r == -EAGAIN, r);

    /* Drain, then leave 1000 bytes of a 4096-byte page... write 5000 when
     * only part fits: partial. */
    while (read(p[0], big, sizeof big) > 0) {
    }
    r = write(p[1], big, 65536 - 1000);
    check("refill", r == 65536 - 1000, r);
    r = write(p[1], big, 5000);
    check("over_pipe_buf_partial", r > 0 && r < 5000, r);
    while (read(p[0], big, sizeof big) > 0) {
    }

    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, 5000);
    check("setpipe_rounds_to_pages", r == 8192, r);
    check("getpipe_after_set", syscall2(SYS_fcntl, p[1], F_GETPIPE_SZ) == 8192, 0);
    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, 3 * 4096);
    check("setpipe_power_of_two_pages", r == 16384, r);
    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, 0);
    check("setpipe_zero_one_page", r == 4096, r);
    r = write(p[1], big, 8192);
    check("one_page_pipe_holds_4096", r == 4096, r);
    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, 65536);
    check("grow_with_data", r == 65536, r);
    write(p[1], big, 8192);
    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, 4096);
    check("shrink_below_contents_ebusy", r == -EBUSY, r);
    r = syscall3(SYS_fcntl, p[0], F_SETPIPE_SZ, -1);
    check("negative_size_einval", r == -EINVAL, r);
    return finish();
}
