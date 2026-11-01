/* readahead on files that live in memory: a file in /dev/shm (tmpfs)
 * and a memfd both return 0 (count 0, a page, past the end), with the
 * same EINVAL for count >= 2^63; the data and the offset are unchanged.
 * A file opened O_RDONLY after being written through another descriptor
 * also works.
 */
#include "a_sys.h"

#define SYS_readahead 187
#define SYS_memfd_create 319

static i64 ra(i64 fd, i64 off, u64 count) { return syscall3(SYS_readahead, fd, off, count); }

static void run(const char *tag, i64 fd) {
    char name[64];
    expect(str_cat2(name, tag, "_write"), a_write(fd, "0123456789abcdef", 16), 16);
    expect(str_cat2(name, tag, "_count0"), ra(fd, 0, 0), 0);
    expect(str_cat2(name, tag, "_page"), ra(fd, 0, 4096), 0);
    expect(str_cat2(name, tag, "_past_eof"), ra(fd, 1 << 24, 4096), 0);
    expect(str_cat2(name, tag, "_huge_count"), ra(fd, 0, 0x8000000000000000UL), -EINVAL);
    expect(str_cat2(name, tag, "_offset"), syscall3(SYS_lseek, fd, 0, SEEK_CUR), 16);
    char b[16];
    memset(b, 0, sizeof b);
    expect(str_cat2(name, tag, "_pread"), syscall4(SYS_pread64, fd, b, 16, 0), 16);
    check(str_cat2(name, tag, "_data"), mem_eq(b, "0123456789abcdef", 16), 0);
}

int test_main(void) {
    const char *shm = "/dev/shm/ld_readahead_memory_files";
    i64 sf = a_open(shm, O_RDWR | O_CREAT | O_TRUNC, 0600);
    run("shm", sf);
    i64 ro = a_open(shm, O_RDONLY, 0);
    expect("shm_rdonly", ra(ro, 0, 16), 0);
    a_close(ro);
    a_close(sf);
    expect("unlink_shm", a_unlink(shm), 0);

    i64 mfd = syscall2(SYS_memfd_create, "ld_readahead", 0);
    check("memfd_open", mfd >= 0, mfd);
    run("memfd", mfd);
    a_close(mfd);
    return finish();
}
