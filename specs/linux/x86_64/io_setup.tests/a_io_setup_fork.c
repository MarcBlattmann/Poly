/* A fork child keeps the ring mapping (same shared pages as the parent:
 * a write by either is seen by the other) but not the context: in the
 * child io_destroy of the parent's handle is EINVAL, and the child's own
 * first context has index 0. The child unmapping its copy does not end
 * the parent's context.
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
    volatile i32 *rep = a_shared();
    u64 a = 0;
    i64 r = io_setup(1, &a);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    volatile u32 *h = (volatile u32 *)a;
    h[2] = 0;

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        rep[0] = h[4] == 0xa10a10a1u;
        h[2] = 55;
        rep[1] = (i32)io_destroy(a);
        u64 own = 0;
        rep[2] = (i32)io_setup(1, &own);
        rep[3] = own != 0 ? (i32)((volatile u32 *)own)[0] : -1;
        rep[4] = (i32)io_destroy(own);
        rep[5] = (i32)syscall2(SYS_munmap, a, 4096);
        syscall1(SYS_exit_group, 0);
    }
    i32 st = -1;
    check("fork_ok", pid > 0 && syscall4(SYS_wait4, pid, &st, 0, 0) == pid, pid);
    check("child_exit_0", st == 0, st);
    check("child_sees_ring", rep[0] == 1, rep[0]);
    check("child_destroy_parent_einval", rep[1] == -EINVAL, rep[1]);
    check("child_own_setup_ok", rep[2] == 0, rep[2]);
    check("child_own_index_0", rep[3] == 0, rep[3]);
    check("child_own_destroy_ok", rep[4] == 0, rep[4]);
    check("child_munmap_ok", rep[5] == 0, rep[5]);
    check("child_write_shared", h[2] == 55, h[2]);
    h[2] = 0;
    r = io_destroy(a);
    check("parent_destroy_ok", r == 0, r);
    return finish();
}
