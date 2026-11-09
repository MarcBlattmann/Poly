/* The /dev/fuse descriptor before and after a mount. Before it is bound to
 * a mount, read and write fail with EPERM. After mounting, the first request
 * (FUSE_INIT) is waiting: poll(POLLIN) reports it, read needs a buffer of
 * at least 8192 bytes (smaller: EINVAL, nothing consumed), returns exactly
 * one request starting with fuse_in_header {len, opcode, unique, ...} where
 * len is the number of bytes read; afterwards poll reports nothing and a
 * non-blocking read is EAGAIN. A write shorter than a fuse_out_header is
 * EINVAL. Two opens are independent descriptors. (linux/fuse.h)
 * poly-diff: needs=root
 * poly-diff: syscalls=off
 */
#include "p_fuse.h"

#define POLLIN 1
struct pollfd_ {
    i32 fd;
    short events, revents;
};

static u8 buf[1 << 17];

int test_main(void) {
    expect("setup", fz_setup("/tmp/pk_fuse_dev"), 0);
    i64 fd = fz_open();
    check("open_dev_fuse", fd >= 0, fd);
    i64 fd2 = a_open("/dev/fuse", O_RDWR, 0);
    check("open_twice", fd2 >= 0 && fd2 != fd, fd2);

    expect("read_unbound", syscall3(SYS_read, fd, buf, sizeof buf), -EPERM);
    expect("write_unbound", syscall3(SYS_write, fd, buf, 16), -EPERM);

    expect("mount", fz_mount(0), 0);
    expect("other_fd_still_unbound", syscall3(SYS_read, fd2, buf, sizeof buf), -EPERM);

    struct pollfd_ pf = {(i32)fd, POLLIN, 0};
    expect("poll_ready", syscall3(SYS_poll, &pf, 1, 1000), 1);
    expect("poll_pollin", pf.revents, POLLIN);

    expect("read_small_100", syscall3(SYS_read, fd, buf, 100), -EINVAL);
    expect("read_small_8191", syscall3(SYS_read, fd, buf, 8191), -EINVAL);
    pf.revents = 0;
    expect("poll_still_ready", syscall3(SYS_poll, &pf, 1, 0), 1);

    i64 n = syscall3(SYS_read, fd, buf, 8192);
    check("read_init_request", n >= 56 && n <= 8192, n);
    const struct fz_in *h = (const void *)buf;
    expect("header_len_is_read_size", h->len, n);
    expect("opcode_init", h->opcode, FZ_INIT);
    expect("nodeid_zero", (i64)h->nodeid, 0);

    pf.revents = 0;
    expect("poll_idle", syscall3(SYS_poll, &pf, 1, 0), 0);
    expect("poll_idle_events", pf.revents, 0);
    syscall3(SYS_fcntl, fd, F_SETFL, O_NONBLOCK);
    expect("read_nonblock_eagain", syscall3(SYS_read, fd, buf, sizeof buf), -EAGAIN);
    expect("write_short_header", syscall3(SYS_write, fd, buf, 8), -EINVAL);

    a_close(fd2);
    a_close(fd);
    syscall2(SYS_umount2, fz_dir, MNT_DETACH);
    a_rmdir(fz_dir);
    return finish();
}
