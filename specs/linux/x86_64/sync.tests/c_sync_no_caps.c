/* sync() needs no privilege: after dropping every capability (capset v3,
 * all sets empty) it still returns 0. Run in a child so the parent keeps
 * its capabilities.
 * poly-diff: syscalls=off (a child process)
 */
#include "a_sys.h"

#define SYS_sync 162
#define SYS_capset_nr 126

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        u32 hdr[2] = {0x20080522, 0};
        u32 data[6] = {0, 0, 0, 0, 0, 0};
        i64 c = syscall2(SYS_capset_nr, hdr, data);
        i64 r = syscall0(SYS_sync);
        exit_group(c == 0 && r == 0 ? 0 : 3);
    }
    i32 status = -1;
    expect("wait", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_ok", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    return finish();
}
