/* A child process sees the same table: the same count and the same
 * index for proc and tmpfs. sysfs needs no privilege.
 * poly-diff: syscalls=names (two processes)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sysfs 139

static i64 sysfs3(i64 opt, i64 a, i64 b) { return syscall3(SYS_sysfs, opt, a, b); }

int test_main(void) {
    i64 n = sysfs3(3, 0, 0);
    i64 p = sysfs3(1, (i64)"proc", 0);
    i64 t = sysfs3(1, (i64)"tmpfs", 0);
    check("parent_ok", n > 0 && p >= 0 && t >= 0, n);
    long pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        int bad = 0;
        if (sysfs3(3, 0, 0) != n) bad |= 1;
        if (sysfs3(1, (i64)"proc", 0) != p) bad |= 2;
        if (sysfs3(1, (i64)"tmpfs", 0) != t) bad |= 4;
        char buf[64];
        memset(buf, 0, sizeof buf);
        if (sysfs3(2, p, (i64)buf) != 0 || !str_eq(buf, "proc")) bad |= 8;
        exit_group(bad);
    }
    check("fork_ok", pid > 0, pid);
    int status = 0;
    i64 w = syscall4(SYS_wait4, pid, &status, 0, 0);
    check("wait_ok", w == pid, w);
    check("child_same_table", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    return finish();
}
