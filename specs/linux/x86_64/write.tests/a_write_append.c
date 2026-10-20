/* write: with O_APPEND every write lands at the current end, whatever the
 * offset; two descriptors appending interleave whole writes. */
#include "a_sys.h"

#define F "/tmp/a_write_append"

int test_main(void) {
    char buf[32];
    expect("setup_file", a_make_file(F, "start", 0644), 0);
    i64 a = a_open(F, O_WRONLY | O_APPEND, 0);
    i64 b = a_open(F, O_WRONLY | O_APPEND, 0);
    i64 plain = a_open(F, O_WRONLY, 0);
    expect("open_three", a + b + plain, 3 + 4 + 5);
    expect("a_write", a_write(a, "-a", 2), 2);
    expect("a_offset", syscall3(SYS_lseek, a, 0, SEEK_CUR), 7);
    expect("b_write", a_write(b, "-b", 2), 2);
    expect("b_offset_is_new_end", syscall3(SYS_lseek, b, 0, SEEK_CUR), 9);
    expect("a_seek_to_0", syscall3(SYS_lseek, a, 0, SEEK_SET), 0);
    expect("a_write_again", a_write(a, "-A", 2), 2);
    expect("a_offset_after_append", syscall3(SYS_lseek, a, 0, SEEK_CUR), 11);
    /* A plain descriptor writes at its own offset. */
    expect("plain_write_at_0", a_write(plain, "S", 1), 1);
    a_close(a);
    a_close(b);
    a_close(plain);
    i64 r = a_open(F, O_RDONLY, 0);
    i64 n = a_read(r, buf, sizeof buf);
    expect("final_length", n, 11);
    check("final_contents", n == 11 && mem_eq(buf, "Start-a-b-A", 11), buf[0]);
    a_close(r);

    /* O_APPEND turned on later with fcntl. */
    i64 fd = a_open(F, O_WRONLY, 0);
    expect("setfl_append", syscall3(SYS_fcntl, fd, F_SETFL, O_APPEND), 0);
    expect("write_after_setfl", a_write(fd, "!", 1), 1);
    expect("offset_after_setfl_write", syscall3(SYS_lseek, fd, 0, SEEK_CUR), 12);
    a_close(fd);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
