/* A fork child's failed io_destroy of the parent's handle does not touch
 * the parent's context: after the child exits the parent destroys it
 * normally. The child's own first context gets index 0 and the child can
 * destroy it.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

int test_main(void) {
    u64 a = 0, b = 0;
    i64 r = io_setup(1, &a);
    check("setup_a_ok", r == 0, r);
    r = io_setup(1, &b);
    check("setup_b_ok", r == 0, r);
    if (!a || !b) return finish();
    a_block_sigchld();
    long pid = a_fork_sync();
    if (pid == 0) {
        int bad = 0;
        u64 c = 0;
        if (io_destroy(a) != -EINVAL) bad = 1;
        if (io_destroy(b) != -EINVAL) bad = 1;
        if (io_setup(1, &c) != 0) bad = 1;
        else {
            if (((volatile u32 *)c)[0] != 0) bad = 1;
            if (io_destroy(c) != 0) bad = 1;
            if (io_destroy(c) != -EINVAL) bad = 1;
        }
        exit_group(bad);
    }
    check("fork_ok", pid > 0, pid);
    i32 st = -1;
    if (pid > 0) syscall4(SYS_wait4, pid, &st, 0, 0);
    check("child_checks_passed", st == 0, st);
    r = io_destroy(a);
    check("parent_a_ok", r == 0, r);
    r = io_destroy(b);
    check("parent_b_ok", r == 0, r);
    return finish();
}
