/* readahead on a regular file opened for reading returns 0, leaves the
 * file offset where it was and changes neither the data nor the size. */
#include "a_sys.h"

#define SYS_readahead 187

static i64 ra(i64 fd, i64 off, i64 count) { return syscall3(SYS_readahead, fd, off, count); }

#define F "/tmp/a_readahead_basic"

static char buf[3 * 4096];

int test_main(void) {
    char got[16];
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    for (int i = 0; i < (int)sizeof buf; i++) buf[i] = (char)('a' + i % 26);
    expect("write", a_write(fd, buf, sizeof buf), sizeof buf);
    i64 r = a_open(F, O_RDONLY, 0);
    expect("whole_file", ra(r, 0, sizeof buf), 0);
    expect("offset_still_0", syscall3(SYS_lseek, r, 0, SEEK_CUR), 0);
    expect("seek_5", syscall3(SYS_lseek, r, 5, SEEK_SET), 5);
    expect("middle", ra(r, 4096, 100), 0);
    expect("unaligned", ra(r, 4097, 3), 0);
    expect("one_byte", ra(r, 0, 1), 0);
    expect("offset_still_5", syscall3(SYS_lseek, r, 0, SEEK_CUR), 5);
    expect("read_after", a_read(r, got, 4), 4);
    check("read_after_data", mem_eq(got, "fghi", 4), got[0]);
    expect("again_same_range", ra(r, 0, sizeof buf), 0);
    struct a_stat st;
    expect("fstat", syscall2(SYS_fstat, r, &st), 0);
    expect("size_unchanged", st.st_size, sizeof buf);
    expect("writer_offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), sizeof buf);
    a_close(r);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
