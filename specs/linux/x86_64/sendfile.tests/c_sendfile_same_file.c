/* in_fd and out_fd may be the same descriptor of a regular file opened
 * O_RDWR. With an offset pointer the bytes are read from *offset and
 * written at the descriptor's file offset, which advances. With NULL the
 * one shared file offset is used for both: the bytes are read from it and
 * written back over themselves, and it advances by the count once.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40

#define F "/tmp/c_sendfile_same_file"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

int test_main(void) {
    char buf[32];
    i64 fd = a_open(F, O_RDWR | O_CREAT | O_TRUNC, 0644);
    expect("setup", a_write(fd, "abcdefgh", 8), 8);
    i64 off = 1;
    expect("copy_to_end", sendfile(fd, fd, &off, 4), 4);
    expect("copy_offset", off, 5);
    expect("copy_file_offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 12);
    expect("copy_pread", syscall4(SYS_pread64, fd, buf, 32, 0), 12);
    check("copy_contents", mem_eq(buf, "abcdefghbcde", 12), buf[8]);

    expect("seek_2", syscall3(SYS_lseek, fd, 2, SEEK_SET), 2);
    expect("null_offset", sendfile(fd, fd, 0, 3), 3);
    expect("null_file_offset", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 5);
    expect("null_pread", syscall4(SYS_pread64, fd, buf, 32, 0), 12);
    check("null_contents_unchanged", mem_eq(buf, "abcdefghbcde", 12), buf[2]);
    a_close(fd);
    expect("cleanup", a_unlink(F), 0);
    return finish();
}
