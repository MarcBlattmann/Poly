/* LOOP_CTL_ADD (0x4C80) makes loop device number `arg` and returns that number; a negative
 * argument picks the lowest number not in use; a number already in use is EEXIST and
 * numbers above 0xfffff are EINVAL. A new device is unbound (LOOP_GET_STATUS64 is ENXIO,
 * its size is 0) and can be bound; LOOP_CTL_REMOVE (0x4C81) removes it (0), after
 * which removing it again is ENODEV.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_lc_add"
#define NUM 1040001 /* a number nobody else uses */

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    expect("add_explicit", p_ioctl(ctl, LOOP_CTL_ADD, NUM), NUM);
    expect("add_explicit_again_eexist", p_ioctl(ctl, LOOP_CTL_ADD, NUM), -EEXIST);
    expect("add_too_big_einval", p_ioctl(ctl, LOOP_CTL_ADD, 0x100000), -EINVAL);
    expect("add_much_too_big_einval", p_ioctl(ctl, LOOP_CTL_ADD, 0x7fffffff), -EINVAL);
    i64 top = p_ioctl(ctl, LOOP_CTL_ADD, 0xfffff);
    check("add_highest_number", top == 0xfffff, top);
    if (top == 0xfffff) expect("remove_highest", p_ioctl(ctl, LOOP_CTL_REMOVE, 0xfffff), 0);
    /* a new device is unbound and empty */
    expect("mknod_block_7", p_node(D "/l", NUM), 0);
    i64 dev = a_open(D "/l", O_RDWR, 0);
    check("open_new_device", dev >= 0, dev);
    struct p_loop_info64 info;
    expect("unbound_get_status64_enxio", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), -ENXIO);
    u64 size = 99;
    expect("unbound_size_ioctl", p_ioctl(dev, BLKGETSIZE64, (i64)&size), 0);
    expect("unbound_size_0", size, 0);
    /* it takes a file */
    char data[2048];
    memset(data, 'x', sizeof data);
    i64 b = p_file(D "/b", data, sizeof data);
    expect("set_fd", p_ioctl(dev, LOOP_SET_FD, b), 0);
    expect("bound_size_ioctl", p_ioctl(dev, BLKGETSIZE64, (i64)&size), 0);
    expect("bound_size_2048", size, 2048);
    expect("clr_fd", p_ioctl(dev, LOOP_CLR_FD, 0), 0);
    a_close(dev);
    expect("remove", p_ioctl(ctl, LOOP_CTL_REMOVE, NUM), 0);
    expect("remove_again_enodev", p_ioctl(ctl, LOOP_CTL_REMOVE, NUM), -ENODEV);
    expect("add_after_remove", p_ioctl(ctl, LOOP_CTL_ADD, NUM), NUM);
    expect("remove_again", p_ioctl(ctl, LOOP_CTL_REMOVE, NUM), 0);
    /* a negative argument: any free number, which then exists */
    i64 a1 = p_ioctl(ctl, LOOP_CTL_ADD, -1);
    check("add_minus1_ge0", a1 >= 0, a1);
    i64 a2 = p_ioctl(ctl, LOOP_CTL_ADD, -7);
    check("add_other_negative_ge0", a2 >= 0 && a2 != a1, a2);
    expect("add_minus1_number_exists", p_ioctl(ctl, LOOP_CTL_ADD, a1), -EEXIST);
    expect("remove_a1", p_ioctl(ctl, LOOP_CTL_REMOVE, a1), 0);
    expect("remove_a2", p_ioctl(ctl, LOOP_CTL_REMOVE, a2), 0);
    a_close(ctl);
    a_close(b);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
