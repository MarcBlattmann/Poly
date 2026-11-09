/* LOOP_CLR_FD while the device is open elsewhere does not unbind it at once: it returns 0,
 * sets LO_FLAGS_AUTOCLEAR and the device stays bound (data readable, LOOP_SET_FD EBUSY)
 * until the last descriptor is closed. LO_FLAGS_AUTOCLEAR can also be switched on and off
 * with LOOP_SET_STATUS64. With only one descriptor open LOOP_CLR_FD unbinds at once.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_la_deferred"

/* open the node again and bind the file (the device already exists and is ours) */
static i64 rebind(const char *node, i64 b) {
    i64 fd = a_open(node, O_RDWR, 0);
    if (fd < 0) return fd;
    i64 r = p_ioctl(fd, LOOP_SET_FD, b);
    if (r < 0) {
        a_close(fd);
        return r;
    }
    return fd;
}

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[4096];
    memset(data, 'k', sizeof data);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 num;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();
    struct p_loop_info64 info;

    /* second opener: CLR_FD is deferred */
    i64 other = a_open(D "/l", O_RDONLY, 0);
    check("second_open", other >= 0, other);
    expect("clr_fd_deferred", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
    expect("still_bound", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("autoclear_set", info.lo_flags, LO_FLAGS_AUTOCLEAR);
    expect("set_fd_ebusy", p_ioctl(dev, LOOP_SET_FD, b), -EBUSY);
    char c = 0;
    expect("read_still_works", syscall4(SYS_pread64, other, &c, 1, 0), 1);
    expect("read_value", c, 'k');
    a_close(dev);
    expect("still_bound_for_other", p_ioctl(other, LOOP_GET_STATUS64, (i64)&info), 0);
    a_close(other);
    dev = a_open(D "/l", O_RDWR, 0);
    expect("unbound_after_last_close", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);
    a_close(dev);

    /* autoclear through the status: set, clear, set */
    dev = rebind(D "/l", b);
    check("set_fd_again", dev >= 0, dev);
    if (dev >= 0) {
        memset(&info, 0, sizeof info);
        info.lo_flags = LO_FLAGS_AUTOCLEAR;
        expect("set_status_autoclear", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
        expect("get_autoclear", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
        expect("flags_autoclear", info.lo_flags, LO_FLAGS_AUTOCLEAR);
        memset(&info, 0, sizeof info);
        expect("set_status_clear", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
        expect("get_cleared", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
        expect("flags_0", info.lo_flags, 0);
        a_close(dev);
        dev = a_open(D "/l", O_RDWR, 0);
        expect("stays_bound_without_autoclear", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
        memset(&info, 0, sizeof info);
        info.lo_flags = LO_FLAGS_AUTOCLEAR;
        expect("set_status_autoclear_2", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
        a_close(dev);
        dev = a_open(D "/l", O_RDWR, 0);
        expect("unbound_after_close", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);
        a_close(dev);
    }

    /* single descriptor: immediate unbind; the same descriptor cannot rebind until reopened */
    dev = rebind(D "/l", b);
    check("set_fd_third", dev >= 0, dev);
    if (dev >= 0) {
        expect("clr_fd_immediate", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
        expect("unbound_at_once", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);
        expect("set_fd_same_descriptor_ebusy", p_ioctl(dev, LOOP_SET_FD, b), -EBUSY);
        a_close(dev);
        dev = a_open(D "/l", O_RDWR, 0);
        expect("set_fd_after_reopen", p_ioctl(dev, LOOP_SET_FD, b), 0);
        p_detach(dev);
    }
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
