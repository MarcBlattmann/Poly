/* getdents takes its descriptor and its count as 32-bit unsigned values:
 * bits above 31 of either register are ignored (count 0x100000018 is 24,
 * fd | 0x100000000 is fd). A count of 0x80000000 or more acts as a
 * buffer too small for any record: EINVAL while there is a record to
 * return, 0 at the end. 0x7fffffff is a valid (large) count. */
#include "a_sys.h"

#define SYS_getdents 78

#define D "/dev/shm/a_getdents_count_bits"

static char buf[4096] __attribute__((aligned(8)));

int test_main(void) {
    expect("setup_mkdir", a_mkdir(D, 0755), 0);
    expect("setup_file", a_make_file(D "/f", "", 0644), 0);
    i64 fd = a_open(D, O_RDONLY | O_DIRECTORY, 0);

    expect("count_high_bits_ignored", syscall3(SYS_getdents, fd, buf, 0x100000018L), 24);
    expect("rewind", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);
    expect("count_0x100000000_is_0", syscall3(SYS_getdents, fd, buf, 0x100000000L), -EINVAL);
    expect("fd_high_bits_ignored", syscall3(SYS_getdents, fd | 0x100000000L, buf, sizeof buf), 72);
    expect("rewind2", syscall3(SYS_lseek, fd, 0, SEEK_SET), 0);

    expect("count_0x80000000", syscall3(SYS_getdents, fd, buf, 0x80000000L), -EINVAL);
    expect("count_0xfffffffe", syscall3(SYS_getdents, fd, buf, 0xfffffffeL), -EINVAL);
    expect("count_0xffffffff", syscall3(SYS_getdents, fd, buf, 0xffffffffL), -EINVAL);
    expect("count_minus_1", syscall3(SYS_getdents, fd, buf, -1L), -EINVAL);
    expect("position_unchanged", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("count_0x7fffffff", syscall3(SYS_getdents, fd, buf, 0x7fffffffL), 72);
    expect("end_count_0x80000000", syscall3(SYS_getdents, fd, buf, 0x80000000L), 0);
    expect("end_count_0xffffffff", syscall3(SYS_getdents, fd, buf, 0xffffffffL), 0);
    a_close(fd);

    expect("cleanup_file", a_unlink(D "/f"), 0);
    expect("cleanup_rmdir", a_rmdir(D), 0);
    return finish();
}
