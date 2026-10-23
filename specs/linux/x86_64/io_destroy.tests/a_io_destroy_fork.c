/* A fork child has a copy of the parent's ring mapping but not the
 * context: io_destroy of the parent's handle in the child is EINVAL.
 * When the parent destroys its context while the child is alive, only
 * the parent's mapping goes: the child's copy stays mapped and readable.
 * The parent's context is not affected by the child's attempt.
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
    u64 a = 0;
    unsigned char v[4];
    int go[2], back[2];
    i64 r = io_setup(1, &a);
    check("setup_ok", r == 0, r);
    if (r != 0) return finish();
    syscall2(SYS_pipe2, go, 0);
    syscall2(SYS_pipe2, back, 0);
    a_block_sigchld();
    long pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        char c = 0;
        syscall1(SYS_close, go[1]);
        syscall1(SYS_close, back[0]);
        syscall3(SYS_read, go[0], &c, 1);
        int bad = 0;
        if (syscall3(SYS_mincore, a, 4096, v) != 0) bad |= 1;
        if (((volatile u32 *)a)[4] != 0xa10a10a1u) bad |= 2;
        if (io_destroy(a) != -EINVAL) bad |= 4;
        if (syscall3(SYS_mincore, a, 4096, v) != 0) bad |= 8;
        c = (char)('0' + bad);
        syscall3(SYS_write, back[1], &c, 1);
        exit_group(0);
    }
    check("fork_ok", pid > 0, pid);
    if (pid <= 0) return finish();
    /* the child's end closed here, so a child that dies reads as EOF */
    syscall1(SYS_close, go[0]);
    syscall1(SYS_close, back[1]);
    r = io_destroy(a);
    check("parent_destroy_ok", r == 0, r);
    r = syscall3(SYS_mincore, a, 4096, v);
    check("parent_ring_gone", r == -ENOMEM, r);
    syscall3(SYS_write, go[1], "x", 1);
    char c = '?';
    syscall3(SYS_read, back[0], &c, 1);
    check("child_copy_mapped", c != '?' && !((c - '0') & 1), c);
    check("child_copy_readable", c != '?' && !((c - '0') & 2), c);
    check("child_destroy_einval", c != '?' && !((c - '0') & 4), c);
    check("child_copy_kept", c != '?' && !((c - '0') & 8), c);
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    check("child_exit_0", st == 0, st);
    return finish();
}
