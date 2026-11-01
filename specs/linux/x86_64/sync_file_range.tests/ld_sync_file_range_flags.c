/* sync_file_range(fd, offset, nbytes, flags) on a regular file: every
 * combination of SYNC_FILE_RANGE_WAIT_BEFORE (1), SYNC_FILE_RANGE_WRITE
 * (2) and SYNC_FILE_RANGE_WAIT_AFTER (4), including none (0), returns 0.
 * Any other bit in the low 32 bits (8, 0x10, 0x80000000, 0xffffffff) is
 * EINVAL; flags is a 32-bit unsigned int, so bits 32..63 are ignored.
 * The data, size and offset of the file are unchanged.
 */
#include "a_sys.h"

#define SYS_sync_file_range 277
/* linux/fs.h */
#define SYNC_FILE_RANGE_WAIT_BEFORE 1
#define SYNC_FILE_RANGE_WRITE 2
#define SYNC_FILE_RANGE_WAIT_AFTER 4

static i64 sfr(i64 fd, i64 off, i64 n, u64 flags) { return syscall4(SYS_sync_file_range, fd, off, n, flags); }

int test_main(void) {
    const char *path = "/tmp/ld_sync_file_range_flags";
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0644);
    char buf[8192];
    for (int i = 0; i < (int)sizeof buf; i++) buf[i] = (char)(i ^ 0x5a);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);

    int ok = 1;
    i64 bad = 0;
    for (u64 f = 0; f <= 7; f++) {
        i64 r = sfr(fd, 0, 0, f);
        if (r != 0) ok = 0, bad = (i64)f * 1000 + r;
    }
    check("flags_0_to_7_whole_file", ok, bad);
    expect("write_wait_range", sfr(fd, 4096, 4096, SYNC_FILE_RANGE_WAIT_BEFORE | SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER), 0);
    expect("write_only_range", sfr(fd, 1, 100, SYNC_FILE_RANGE_WRITE), 0);
    expect("flag_8", sfr(fd, 0, 0, 8), -EINVAL);
    expect("flag_0x10", sfr(fd, 0, 0, 0x10), -EINVAL);
    expect("flag_write_and_8", sfr(fd, 0, 0, SYNC_FILE_RANGE_WRITE | 8), -EINVAL);
    expect("flag_bit31", sfr(fd, 0, 0, 0x80000000UL), -EINVAL);
    expect("flag_all_32", sfr(fd, 0, 0, 0xffffffffUL), -EINVAL);
    expect("flag_high_bits_ignored", sfr(fd, 0, 0, (1UL << 32) | SYNC_FILE_RANGE_WRITE), 0);
    expect("flag_high_bits_8", sfr(fd, 0, 0, (1UL << 40) | 8), -EINVAL);

    expect("offset_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), sizeof buf);
    struct a_stat st;
    syscall2(SYS_fstat, fd, &st);
    expect("size_unchanged", st.st_size, sizeof buf);
    char back[8192];
    memset(back, 0, sizeof back);
    expect("pread", syscall4(SYS_pread64, fd, back, sizeof back, 0), sizeof back);
    check("data_intact", mem_eq(back, buf, sizeof buf), 0);

    a_close(fd);
    expect("unlink", a_unlink(path), 0);
    return finish();
}
