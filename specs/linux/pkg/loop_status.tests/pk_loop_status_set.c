/* LOOP_SET_STATUS64 (0x4C04) on a bound device. lo_offset and lo_sizelimit change which part
 * of the file the device shows: size = (min(file size - offset, sizelimit if non-zero)
 * rounded down to 512-byte sectors, 0 when the offset is at or beyond the end); offsets need
 * not be aligned and the device's byte 0 is the file's byte at the offset. The values are
 * stored as given (a limit larger than the file is kept). Every call replaces all of them:
 * fields left 0 reset the offset and limit. lo_file_name is stored with the last byte forced
 * to 0. The flags LO_FLAGS_AUTOCLEAR (4) and LO_FLAGS_PARTSCAN (8) can be set; AUTOCLEAR is
 * cleared by a call that does not name it, PARTSCAN stays once set; LO_FLAGS_READ_ONLY and
 * unknown bits are ignored. lo_encrypt_type != 0 and lo_encrypt_key_size > 32 are EINVAL;
 * lo_number, lo_device, lo_inode and lo_rdevice in the argument are ignored; a NULL
 * argument is EFAULT.
 * poly-diff: needs=root (mknod)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_ls_set"

static u64 devsize(i64 d) {
    u64 s = 77;
    if (p_ioctl(d, BLKGETSIZE64, (i64)&s) < 0) return (u64)-1;
    return s;
}

static i64 set(i64 dev, u64 off, u64 lim, u32 flags) {
    struct p_loop_info64 info;
    memset(&info, 0, sizeof info);
    info.lo_offset = off;
    info.lo_sizelimit = lim;
    info.lo_flags = flags;
    return p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info);
}

int test_main(void) {
    expect("mkdir", p_mkdir(D), 0);
    i64 ctl = p_ctl_open(D "/ctl");
    check("open_control", ctl >= 0, ctl);
    static char data[16384];
    for (int i = 0; i < 16384; i++) data[i] = (char)('a' + (i / 512) % 26);
    i64 b = p_file(D "/b", data, sizeof data);
    i64 num = -1;
    i64 dev = p_attach(ctl, b, D "/l", &num, 0);
    check("set_fd", dev >= 0, dev);
    if (dev < 0) return finish();
    struct p_loop_info64 info;
    char c = 0;

    expect("size_16384", devsize(dev), 16384);
    expect("offset_512", set(dev, 512, 0, 0), 0);
    expect("size_15872", devsize(dev), 15872);
    expect("flush", p_ioctl(dev, BLKFLSBUF, 0), 0);
    expect("read_first", syscall4(SYS_pread64, dev, &c, 1, 0), 1);
    expect("first_byte_b", c, 'b');
    expect("offset_100_unaligned", set(dev, 100, 0, 0), 0);
    expect("size_31_sectors", devsize(dev), 15872);
    expect("flush_2", p_ioctl(dev, BLKFLSBUF, 0), 0);
    expect("read_first_2", syscall4(SYS_pread64, dev, &c, 1, 0), 1);
    expect("first_byte_a", c, 'a');
    expect("read_at_412", syscall4(SYS_pread64, dev, &c, 1, 412), 1);
    expect("byte_412_is_b", c, 'b'); /* file byte 512 */
    expect("offset_past_end", set(dev, 20000, 0, 0), 0);
    expect("size_0", devsize(dev), 0);
    expect("limit_1000", set(dev, 0, 1000, 0), 0);
    expect("size_one_sector", devsize(dev), 512);
    expect("offset_1024_limit_4096", set(dev, 1024, 4096, 0), 0);
    expect("size_4096", devsize(dev), 4096);
    expect("limit_larger_than_file", set(dev, 1024, 100000, 0), 0);
    expect("size_15360", devsize(dev), 15360);
    expect("get_status64", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("offset_kept", info.lo_offset, 1024);
    expect("limit_kept_as_given", info.lo_sizelimit, 100000);
    expect("reset_all_zero", set(dev, 0, 0, 0), 0);
    expect("size_back_16384", devsize(dev), 16384);
    expect("get_status64_2", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("offset_reset", info.lo_offset, 0);
    expect("limit_reset", info.lo_sizelimit, 0);

    /* the name */
    memset(&info, 0, sizeof info);
    str_cpy((char *)info.lo_file_name, "myname");
    expect("set_name", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
    memset(&info, 0x55, sizeof info);
    expect("get_name", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    check("name_is_myname", str_eq((char *)info.lo_file_name, "myname"), 0);
    memset(&info, 0, sizeof info);
    memset(info.lo_file_name, 'N', 64);
    expect("set_name_64", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
    memset(&info, 0x55, sizeof info);
    expect("get_name_64", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("name_byte_62", info.lo_file_name[62], 'N');
    expect("name_byte_63_nul", info.lo_file_name[63], 0);

    /* flags */
    expect("set_read_only_flag", set(dev, 0, 0, LO_FLAGS_READ_ONLY), 0);
    expect("get_flags_1", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("read_only_flag_ignored", info.lo_flags, 0);
    expect("device_stays_writable", syscall4(SYS_pwrite64, dev, "x", 1, 0), 1);
    expect("set_unknown_bits", set(dev, 0, 0, 0x80 | 0x40000000), 0);
    expect("get_flags_2", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("unknown_bits_ignored", info.lo_flags, 0);
    expect("set_autoclear", set(dev, 0, 0, LO_FLAGS_AUTOCLEAR), 0);
    expect("get_flags_3", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("autoclear_set", info.lo_flags, LO_FLAGS_AUTOCLEAR);
    expect("set_partscan", set(dev, 0, 0, LO_FLAGS_PARTSCAN), 0);
    expect("get_flags_4", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("partscan_set_autoclear_cleared", info.lo_flags, LO_FLAGS_PARTSCAN);
    expect("set_none_partscan_stays", set(dev, 0, 0, 0), 0);
    expect("get_flags_5", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("partscan_stays", info.lo_flags, LO_FLAGS_PARTSCAN);
    expect("set_all", set(dev, 0, 0, LO_FLAGS_PARTSCAN | LO_FLAGS_AUTOCLEAR | LO_FLAGS_READ_ONLY), 0);
    expect("get_flags_6", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("autoclear_and_partscan", info.lo_flags, LO_FLAGS_PARTSCAN | LO_FLAGS_AUTOCLEAR);
    expect("clear_autoclear", set(dev, 0, 0, 0), 0);

    /* rejected and ignored fields */
    memset(&info, 0, sizeof info);
    info.lo_encrypt_type = 1;
    expect("encrypt_type_einval", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), -EINVAL);
    info.lo_encrypt_type = 0;
    info.lo_encrypt_key_size = 33;
    expect("key_size_33_einval", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), -EINVAL);
    info.lo_encrypt_key_size = 0;
    info.lo_number = 999;
    info.lo_device = 5;
    info.lo_inode = 6;
    info.lo_rdevice = 7;
    expect("identity_fields_ignored", p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info), 0);
    expect("get_status64_3", p_ioctl(dev, LOOP_GET_STATUS64, (i64)&info), 0);
    expect("number_unchanged", info.lo_number, num);
    check("device_unchanged", info.lo_device != 5 && info.lo_inode != 6, 0);
    expect("null_argument_efault", p_ioctl(dev, LOOP_SET_STATUS64, 0), -EFAULT);

    p_detach(dev);
    a_close(b);
    a_close(ctl);
    a_unlink(D "/b");
    a_unlink(D "/l");
    a_unlink(D "/ctl");
    expect("rmdir", a_rmdir(D), 0);
    return finish();
}
