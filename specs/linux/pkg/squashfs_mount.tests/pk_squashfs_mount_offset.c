/* An image that sits inside a larger file (AppImage style: a program followed by its
 * squashfs image) is mounted through a loop device whose offset points at the image.
 * Without the offset the device starts with the program's bytes and mount fails with EINVAL;
 * with lo_offset set by LOOP_SET_STATUS64 after LOOP_SET_FD, or by LOOP_CONFIGURE, the mount
 * works at any byte offset (3000, 100003, 4096), not only at multiples of the sector size;
 * an offset one byte off is EINVAL. A size limit that cuts the image (512 or 100 bytes of a
 * 493-byte image: the file system reads whole 4096-byte blocks) is EIO, one that covers it works.
 * Changing the offset of a mounted device back to 0 is allowed and does not disturb the
 * mount. Image: small-gzip, preceded by bytes of value 0x5a.
 * poly-diff: needs=root (loop device, mount)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_sm_offset"

static const unsigned char img[] = {
    104,115,113,115,6,0,0,0,0,0,0,0,0,16,0,0,1,0,0,0,1,0,12,0,
    192,0,1,0,4,0,0,0,173,0,0,0,0,0,0,0,237,1,0,0,0,0,0,0,
    229,1,0,0,0,0,0,0,255,255,255,255,255,255,255,255,0,1,0,0,0,0,0,0,
    94,1,0,0,0,0,0,0,183,1,0,0,0,0,0,0,215,1,0,0,0,0,0,0,
    120,218,237,200,193,17,192,16,0,0,176,191,105,40,165,198,161,116,255,17,236,209,75,158,
    137,233,202,229,174,237,233,99,190,107,127,33,10,33,132,16,66,8,33,132,16,226,111,113,
    0,154,176,41,223,120,218,237,200,71,17,192,32,0,0,176,63,106,104,161,3,57,76,255,
    18,240,193,37,207,132,120,221,41,63,239,247,151,218,250,152,43,8,33,132,16,66,8,33,
    132,16,226,180,216,16,225,41,131,120,218,75,227,50,48,52,50,54,49,53,51,183,176,76,
    76,74,78,73,77,27,21,24,21,24,21,64,23,0,0,42,156,209,170,105,110,110,101,114,
    10,104,101,108,108,111,32,115,113,117,97,115,104,102,115,10,92,0,120,218,99,98,88,194,
    200,0,5,76,12,168,128,13,136,25,25,222,194,229,25,145,212,41,64,229,153,144,244,51,
    163,233,229,7,139,253,135,203,179,64,205,224,4,226,140,212,156,156,124,189,146,138,18,100,
    3,88,129,56,1,136,255,3,1,136,175,161,204,192,96,10,164,77,128,88,9,205,49,108,
    72,150,186,50,200,50,176,3,105,0,164,48,14,121,69,0,120,218,99,96,128,0,38,56,
    205,193,144,153,151,151,90,164,87,82,81,194,12,21,100,4,98,5,40,157,226,0,84,3,
    82,149,145,154,147,147,15,82,149,192,192,204,0,82,153,211,200,192,2,148,225,98,40,72,
    44,41,73,45,202,3,201,1,0,224,50,14,29,16,128,235,0,0,0,0,0,0,0,21,
    0,0,1,0,0,0,0,165,1,0,0,0,0,0,0,22,0,120,218,83,96,64,5,14,
    80,58,1,74,55,66,233,181,80,26,0,36,168,1,239,191,1,0,0,0,0,0,0,4,
    128,0,0,0,0,223,1,0,0,0,0,0,0,
};

static struct p_sq sq;
static char pb[160];

static i64 set_offset(i64 dev, u64 off, u64 lim) {
    struct p_loop_info64 info;
    memset(&info, 0, sizeof info);
    info.lo_offset = off;
    info.lo_sizelimit = lim;
    return p_ioctl(dev, LOOP_SET_STATUS64, (i64)&info);
}

static void read_hello(const char *what) {
    char buf[32];
    memset(buf, 0, sizeof buf);
    i64 fd = a_open(p_sq_path(&sq, pb, "/hello.txt"), O_RDONLY, 0);
    i64 n = fd < 0 ? fd : a_read(fd, buf, sizeof buf);
    check(what, n == 15 && mem_eq(buf, "hello squashfs\n", 15), n);
    if (fd >= 0) a_close(fd);
}

int test_main(void) {
    /* image after 3000 bytes of something else */
    check("setup", p_sq_setup(&sq, D, img, sizeof img, 3000, 0) == 0, 0);
    expect("no_offset_einval", p_sq_mount(&sq), -EINVAL);
    expect("offset_3001_einval", set_offset(sq.dev, 3001, 0), 0);
    expect("mount_at_3001_einval", p_sq_mount(&sq), -EINVAL);
    expect("offset_2999_einval", set_offset(sq.dev, 2999, 0), 0);
    expect("mount_at_2999_einval", p_sq_mount(&sq), -EINVAL);
    expect("offset_3000", set_offset(sq.dev, 3000, 0), 0);
    u64 size = 0;
    p_ioctl(sq.dev, BLKGETSIZE64, (i64)&size);
    expect("device_size_1024", size, 1024); /* file 4096 - offset 3000, whole sectors */
    expect("mount_at_3000", p_sq_mount(&sq), 0);
    read_hello("hello_through_offset");
    struct a_statfs fs;
    expect("statfs", syscall2(SYS_statfs, sq.mnt, &fs), 0);
    expect("magic", fs.f_type, 0x73717368);
    expect("umount", p_umount(sq.mnt), 0);

    /* size limits */
    expect("limit_100", set_offset(sq.dev, 3000, 100), 0);
    expect("mount_limit_100_eio", p_sq_mount(&sq), -EIO);
    expect("limit_512", set_offset(sq.dev, 3000, 512), 0);
    expect("mount_limit_512_eio", p_sq_mount(&sq), -EIO);
    expect("limit_exact_page", set_offset(sq.dev, 3000, 4096), 0);
    expect("mount_limit_4096", p_sq_mount(&sq), 0);
    read_hello("hello_with_limit");
    expect("umount_2", p_umount(sq.mnt), 0);
    expect("limit_larger_than_file", set_offset(sq.dev, 3000, 1 << 20), 0);
    expect("mount_big_limit", p_sq_mount(&sq), 0);

    /* the mounted device keeps working when its offset is reset */
    read_hello("hello_before_reset");
    expect("reset_offset_while_mounted", set_offset(sq.dev, 0, 0), 0);
    read_hello("hello_after_reset");
    expect("umount_3", p_umount(sq.mnt), 0);
    p_sq_teardown(&sq);

    /* a large, odd offset set with LOOP_CONFIGURE in one step */
    struct p_loop_info64 cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.lo_offset = 100003;
    cfg.lo_flags = LO_FLAGS_READ_ONLY;
    p_sq_cfg = &cfg;
    check("setup_2_with_loop_configure", p_sq_setup(&sq, D, img, sizeof img, 100003, 0) == 0, 0);
    p_sq_cfg = 0;
    expect("mount_configured", p_sq_mount(&sq), 0);
    read_hello("hello_configured");
    expect("umount_4", p_umount(sq.mnt), 0);
    expect("offset_4096_is_not_the_image", set_offset(sq.dev, 4096, 0), 0);
    expect("mount_wrong_aligned_offset_einval", p_sq_mount(&sq), -EINVAL);
    p_sq_teardown(&sq);
    return finish();
}
