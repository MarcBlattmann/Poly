/* Images that must not mount. The superblock (little-endian): magic u32 at 0 (0x73717368),
 * inode count u32 at 4, block size u32 at 12, compression id u16 at 20 (1 gzip, 2 lzma, 3 lzo,
 * 4 xz, 5 lz4, 6 zstd), block log u16 at 22, major version u16 at 28 (4), minor u16 at 30,
 * root inode reference u64 at 32, bytes used u64 at 40. mount() fails with EINVAL for: a
 * wrong magic; a major version other than 4; a block log that is not log2 of the block
 * size; a block size that is not a power of two or is below 4096 or above 1 MiB; a
 * compression id that is 0, unknown (99) or the legacy lzma (2); bytes used far larger than
 * the device (1 GiB); a zero-filled device. A device that is shorter than 4096 bytes (a
 * 512-byte file holding the start of a 493-byte image: the file system reads in blocks of
 * 4096 bytes) is EIO, and so is a
 * device of size 0 (a file of 0, 100 or 511 bytes). A source that is a
 * regular file is ENOTBLK and a missing one ENOENT. After every failure the device is still
 * bound and can be detached.
 * Image: small-gzip.
 * poly-diff: needs=root (loop device, mount)
 * poly-diff: syscalls=off (device numbers are different on every run)
 */
#include "p_loop.h"

#define D "/tmp/pk_sm_bad"

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

static unsigned char work[8192];
static struct p_sq sq;
static int raw;

static void put16(size_t off, u16 v) {
    work[off] = (unsigned char)v;
    work[off + 1] = (unsigned char)(v >> 8);
}
static void put32(size_t off, u32 v) {
    put16(off, (u16)v);
    put16(off + 2, (u16)(v >> 16));
}
static void put64(size_t off, u64 v) {
    put32(off, (u32)v);
    put32(off + 4, (u32)(v >> 32));
}

/* mount a (modified) copy of the image whose file is `len` bytes long; the result of mount */
static i64 try_mount(const char *name, size_t len) {
    i64 r = raw ? p_sq_setup_raw(&sq, D, work, len) : p_sq_setup(&sq, D, work, len, 0, 0);
    if (r < 0) {
        check("setup_for_case", 0, r);
        p_sq_teardown(&sq);
        return r;
    }
    i64 m = p_sq_mount(&sq);
    check(name, 1, 0);
    if (m == 0) p_umount(sq.mnt);
    struct p_loop_info64 info;
    expect("device_still_bound", p_ioctl(sq.dev, LOOP_GET_STATUS64, (i64)&info), 0);
    p_sq_teardown(&sq);
    return m;
}

static void fresh(void) {
    memset(work, 0, sizeof work);
    memcpy(work, img, sizeof img);
}

int test_main(void) {
    fresh();
    expect("unmodified_mounts", try_mount("case_unmodified", sizeof img), 0);

    fresh();
    work[0] = 'X';
    expect("bad_magic_einval", try_mount("case_magic", sizeof img), -EINVAL);
    fresh();
    put32(0, 0x68737173); /* big-endian magic of other-endian images */
    expect("swapped_magic_einval", try_mount("case_swapped_magic", sizeof img), -EINVAL);
    fresh();
    put16(28, 3);
    expect("major_3_einval", try_mount("case_major_3", sizeof img), -EINVAL);
    fresh();
    put16(28, 5);
    expect("major_5_einval", try_mount("case_major_5", sizeof img), -EINVAL);
    fresh();
    put16(22, 13);
    expect("block_log_mismatch_einval", try_mount("case_block_log", sizeof img), -EINVAL);
    fresh();
    put32(12, 5000);
    expect("block_size_not_power_of_two_einval", try_mount("case_block_5000", sizeof img), -EINVAL);
    fresh();
    put32(12, 2048);
    put16(22, 11);
    expect("block_size_2048_einval", try_mount("case_block_2048", sizeof img), -EINVAL);
    fresh();
    put32(12, 2 * 1024 * 1024);
    put16(22, 21);
    expect("block_size_2m_einval", try_mount("case_block_2m", sizeof img), -EINVAL);
    fresh();
    put16(20, 0);
    expect("compression_0_einval", try_mount("case_comp_0", sizeof img), -EINVAL);
    fresh();
    put16(20, 99);
    expect("compression_99_einval", try_mount("case_comp_99", sizeof img), -EINVAL);
    fresh();
    put16(20, 2);
    expect("compression_lzma_einval", try_mount("case_comp_lzma", sizeof img), -EINVAL);
    fresh();
    put64(40, 1UL << 30);
    expect("bytes_used_too_big_einval", try_mount("case_bytes_used", sizeof img), -EINVAL);

    /* cut off: the image is 493 bytes, the file has 512 */
    fresh();
    raw = 1;
    expect("short_device_eio", try_mount("case_short", 512), -EIO);
    raw = 0;
    /* a zero-filled device */
    memset(work, 0, sizeof work);
    expect("zero_filled_einval", try_mount("case_zero", 4096), -EINVAL);
    /* too small to hold a sector */
    fresh();
    raw = 1;
    expect("device_size_0_eio", try_mount("case_empty", 0), -EIO);
    fresh();
    expect("device_size_100_bytes_eio", try_mount("case_100_bytes", 100), -EIO);
    expect("device_size_511_bytes_eio", try_mount("case_511_bytes", 511), -EIO);
    raw = 0;

    /* sources that are not loop devices holding images */
    check("setup_plain", p_sq_setup(&sq, D, img, sizeof img, 0, 0) == 0, 0);
    expect("regular_file_enotblk", p_mount_squash(sq.imgp, sq.mnt, 0), -ENOTBLK);
    char missing[96];
    str_cat2(missing, D, "/nodev");
    expect("missing_source_enoent", p_mount_squash(missing, sq.mnt, 0), -ENOENT);
    expect("directory_source_enotblk", p_mount_squash(D, sq.mnt, 0), -ENOTBLK);
    expect("char_device_source_enotblk", p_mount_squash(sq.ctlp, sq.mnt, 0), -ENOTBLK);
    p_sq_teardown(&sq);
    return finish();
}
