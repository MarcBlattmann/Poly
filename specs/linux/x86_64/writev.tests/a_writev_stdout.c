/* writev: to standard output, several buffers make one line each; the
 * result is the byte count. */
#include "a_sys.h"

int test_main(void) {
    struct a_iovec iov[5] = {{"ok ", 3}, {"writev_", 7}, {"", 0}, {"to_stdout", 9}, {"\n", 1}};
    i64 n = syscall3(SYS_writev, 1, iov, 5);
    passed++;
    expect("writev_stdout_count", n, 20);
    struct a_iovec line[3] = {{"ok wri", 6}, {"tev_second_line", 15}, {"\n", 1}};
    n = syscall3(SYS_writev, 1, line, 3);
    passed++;
    expect("second_count", n, 22);
    struct a_iovec err[2] = {{"writev to stderr", 16}, {"\n", 1}};
    expect("writev_stderr_count", syscall3(SYS_writev, 2, err, 2), 17);
    return finish();
}
