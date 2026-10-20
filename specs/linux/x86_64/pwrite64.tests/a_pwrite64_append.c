/* pwrite64: on a descriptor opened with O_APPEND, Linux appends the data
 * whatever the offset (a documented deviation from POSIX). */
#include "a_sys.h"

#define F "/tmp/a_pwrite64_append"

static i64 pwrite(i64 fd, const void *buf, size_t n, i64 off) { return syscall4(SYS_pwrite64, fd, buf, n, off); }

int test_main(void) {
    char buf[32];
    struct a_stat st;
    expect("setup_file", a_make_file(F, "base", 0644), 0);
    i64 fd = a_open(F, O_RDWR | O_APPEND, 0);
    expect("open_append", fd, 3);
    expect("pwrite_at_0_appends", pwrite(fd, "AB", 2, 0), 2);
    syscall2(SYS_fstat, fd, &st);
    expect("size_6", st.st_size, 6);
    expect("offset_unchanged_0", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 0);
    expect("pwrite_far_appends_too", pwrite(fd, "C", 1, 1000), 1);
    syscall2(SYS_fstat, fd, &st);
    expect("size_7_no_hole", st.st_size, 7);
    expect("pread", syscall4(SYS_pread64, fd, buf, sizeof buf, 0), 7);
    check("contents", mem_eq(buf, "baseABC", 7), buf[0]);
    /* Without O_APPEND (cleared with F_SETFL) the offset is honoured. */
    expect("clear_append", syscall3(SYS_fcntl, fd, F_SETFL, 0), 0);
    expect("pwrite_at_0_overwrites", pwrite(fd, "B", 1, 0), 1);
    syscall2(SYS_fstat, fd, &st);
    expect("size_still_7", st.st_size, 7);
    syscall4(SYS_pread64, fd, buf, 1, 0);
    check("first_byte_overwritten", buf[0] == 'B', buf[0]);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
