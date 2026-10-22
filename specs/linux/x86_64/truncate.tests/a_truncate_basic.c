/* truncate: by path, shrinking discards the data past the new length and
 * growing adds zero bytes (data cut off once does not come back); the
 * call returns 0. Truncating to the current size and to 0 succeeds. */
#include "a_sys.h"

#define F "/tmp/a_truncate_basic"

int test_main(void) {
    struct a_stat st;
    char buf[32];
    expect("setup_file", a_make_file(F, "hello world", 0644), 0);
    expect("shrink", syscall2(SYS_truncate, F, 5), 0);
    a_stat_path(F, &st);
    expect("shrink_size", st.st_size, 5);
    i64 fd = a_open(F, O_RDONLY, 0);
    expect("shrink_read", a_read(fd, buf, sizeof buf), 5);
    check("shrink_data_kept", mem_eq(buf, "hello", 5), buf[0]);
    a_close(fd);
    expect("grow", syscall2(SYS_truncate, F, 12), 0);
    a_stat_path(F, &st);
    expect("grow_size", st.st_size, 12);
    for (int i = 0; i < 32; i++) buf[i] = 'x';
    fd = a_open(F, O_RDONLY, 0);
    expect("grow_read", a_read(fd, buf, sizeof buf), 12);
    a_close(fd);
    check("grow_data_kept", mem_eq(buf, "hello", 5), buf[0]);
    int zeros = 1;
    for (int i = 5; i < 12; i++)
        if (buf[i]) zeros = 0;
    check("grow_zero_filled", zeros, buf[5]);
    expect("same_size", syscall2(SYS_truncate, F, 12), 0);
    a_stat_path(F, &st);
    expect("same_size_size", st.st_size, 12);
    expect("to_zero", syscall2(SYS_truncate, F, 0), 0);
    a_stat_path(F, &st);
    expect("to_zero_size", st.st_size, 0);
    expect("zero_again", syscall2(SYS_truncate, F, 0), 0);
    expect("grow_from_zero", syscall2(SYS_truncate, F, 3), 0);
    fd = a_open(F, O_RDONLY, 0);
    buf[0] = buf[1] = buf[2] = 'x';
    expect("grow_from_zero_read", a_read(fd, buf, sizeof buf), 3);
    check("grow_from_zero_zeros", buf[0] == 0 && buf[1] == 0 && buf[2] == 0, buf[0]);
    a_close(fd);
    a_stat_path(F, &st);
    expect("still_regular", st.st_mode & S_IFMT, S_IFREG);
    expect("mode_unchanged", st.st_mode & 07777, 0644);
    expect("cleanup_unlink", a_unlink(F), 0);
    return finish();
}
