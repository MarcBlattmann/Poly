/* sync() needs no open descriptors: a child that has closed 0, 1 and 2
 * still gets 0. The child reports through its exit status.
 * poly-diff: syscalls=off (a child process)
 */
#include "a_sys.h"

#define SYS_sync 162

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        a_close(0);
        a_close(1);
        a_close(2);
        i64 r = syscall0(SYS_sync);
        i64 w = syscall3(SYS_write, 1, "x", 1);
        exit_group(r == 0 && w == -EBADF ? 0 : 3);
    }
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_ok", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    expect("parent", syscall0(SYS_sync), 0);
    return finish();
}
