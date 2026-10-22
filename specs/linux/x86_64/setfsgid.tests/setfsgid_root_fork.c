/* The filesystem gid is inherited across fork and is per process: a
 * child's change is not seen by its parent, and the reverse.
 * poly-diff: syscalls=off (futex gate retries depend on scheduling)
 * poly-diff: needs=root (setting other gids needs CAP_SETGID)
 */
#include "poly_ipc.h"

int test_main(void) {
    syscall1(SYS_setfsgid, 812);
    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        int ok = syscall1(SYS_setfsgid, 813) == 812;
        gate_open(&g[1]);
        gate_wait(&g[2]);
        ok = ok && syscall1(SYS_setfsgid, -1) == 813;
        exit_group(ok ? 0 : 1);
    }
    gate_wait(&g[1]);
    check("parent_keeps_after_child_change", syscall1(SYS_setfsgid, -1) == 812, syscall1(SYS_setfsgid, -1));
    syscall1(SYS_setfsgid, 814);
    gate_open(&g[2]);
    check("child_inherits_and_keeps_own", wait_exit(pid) == 0, 0);
    check("parent_now_814", syscall1(SYS_setfsgid, -1) == 814, 0);
    return finish();
}
