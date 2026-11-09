/* /dev/loop-control (character device 10:237): LOOP_CTL_GET_FREE (0x4C82) returns the
 * number of a loop device that is not bound (the argument is ignored), making one if
 * needed; a device bound to a file is never returned; reads, writes and seeks on the
 * control device fail; an unknown request is ENOSYS.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lc_getfree"

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    struct a_stat st;
    expect("fstat", syscall2(SYS_fstat, ctl, &st), 0);
    expect("mode_chr_0600", st.st_mode, S_IFCHR | 0600);
    expect("rdev_10_237", st.st_rdev, p_dev(10, 237));
    i64 n = p_ioctl(ctl, LOOP_CTL_GET_FREE, 0);
    check("get_free_ge_0", n >= 0, n);
    check("get_free_arg_ignored", p_ioctl(ctl, LOOP_CTL_GET_FREE, 12345) >= 0, 0);
    check("get_free_fits_minor", n < (1 << 20), n);
    /* bind a file to the device we were given: it is no longer free */
    char data[1024];
    memset(data, 'd', sizeof data);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 num = -1;
    i64 dev = p_attach_free(ctl, b, D "/l", &num, 0);
    check("attach", dev >= 0, dev);
    if (dev >= 0) {
        for (int i = 0; i < 4; i++) {
            i64 f = p_ioctl(ctl, LOOP_CTL_GET_FREE, 0);
            check("bound_device_not_free", f >= 0 && f != num, f);
        }
        p_detach(dev);
    }
    /* the control device is not a data device */
    char c[4];
    expect("read_einval", a_read(ctl, c, 4), -EINVAL);
    expect("write_einval", a_write(ctl, "x", 1), -EINVAL);
    expect("lseek_espipe", syscall3(SYS_lseek, ctl, 0, SEEK_SET), -ESPIPE);
    expect("unknown_request_enosys", p_ioctl(ctl, 0x4C83, 0), -ENOSYS);
    expect("loop_set_fd_on_control_enosys", p_ioctl(ctl, LOOP_SET_FD, 0), -ENOSYS);
    expect("tty_request_enosys", p_ioctl(ctl, 0x5401, 0), -ENOSYS);
    /* opened read-only it works the same */
    i64 ro = a_open(D "/ctl", O_RDONLY, 0);
    check("open_ro", ro >= 0, ro);
    check("get_free_via_ro_fd", p_ioctl(ro, LOOP_CTL_GET_FREE, 0) >= 0, 0);
    a_close(ro);
    a_close(ctl);
    a_close(b);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
