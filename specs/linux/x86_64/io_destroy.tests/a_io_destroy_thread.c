/* A task sharing the address space (CLONE_VM) can destroy a context the
 * parent made: the ring disappears for the parent too, and the parent's
 * own io_destroy of it is then EINVAL. The parent can destroy a context
 * the child made.
 * poly-diff: syscalls=off (two tasks)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

static u64 parent_ctx, child_ctx;
static volatile i64 child_destroy, child_setup;

static int destroyer(void *arg) {
    (void)arg;
    child_destroy = io_destroy(parent_ctx);
    return 0;
}

static int maker(void *arg) {
    (void)arg;
    child_setup = io_setup(1, &child_ctx);
    return 0;
}

static void run(int (*fn)(void *)) {
    static volatile i32 done;
    long pid = a_spawn(0, &done, fn, 0);
    check("spawn_ok", pid > 0, pid);
    if (pid > 0) {
        while (done != 0) syscall0(SYS_sched_yield);
        i32 st;
        syscall4(SYS_wait4, pid, &st, 0, 0);
    }
}

int test_main(void) {
    unsigned char v[4];
    i64 r = io_setup(1, &parent_ctx);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    a_block_sigchld();
    run(destroyer);
    check("child_destroy_ok", child_destroy == 0, child_destroy);
    r = syscall3(SYS_mincore, parent_ctx, 4096, v);
    check("ring_gone_for_parent", r == -ENOMEM, r);
    r = io_destroy(parent_ctx);
    check("parent_destroy_einval", r == -EINVAL, r);
    run(maker);
    check("child_setup_ok", child_setup == 0, child_setup);
    r = io_destroy(child_ctx);
    check("parent_destroys_child_ctx", r == 0, r);
    r = io_destroy(child_ctx);
    check("again_einval", r == -EINVAL, r);
    return finish();
}
