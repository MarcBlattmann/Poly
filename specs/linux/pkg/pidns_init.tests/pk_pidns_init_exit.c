/* When pid 1 of a PID namespace exits, every other process of the
 * namespace is killed with SIGKILL (a process that was blocked in read
 * ends "killed by 9"; here a grandchild reports it through a pipe whose
 * read end the outer test holds: it sees end-of-file once all of them are
 * gone), including a process that had been reparented to pid 1. The
 * outer parent sees pid 1's own exit status. A process in the namespace
 * that was created by a thread of pid 1 (clone with CLONE_THREAD) and
 * outlives the main thread's exit_group is also killed. PR_SET_CHILD_SUBREAPER
 * inside the namespace on a non-init process takes the orphans of its
 * descendants: a grandchild whose parent exited reads getppid() equal to
 * the subreaper's pid (not 1), and PR_GET_CHILD_SUBREAPER reads 1 for it.
 * poly-diff: needs=root (a new PID namespace needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static i32 alive[2], report[2], blk[2];

/* In pid 1: start two helpers that block (the second through a
 * grandchild), then exit with 7. */
static int init_exits(void) {
    for (int i = 0; i < 2; i++) {
        i64 c = p_fork();
        if (c == 0) {
            if (i == 1) {
                /* a grandchild: its parent exits, it is reparented to pid 1 */
                i32 link[2];
                p_pipe(link);
                i64 g = p_fork();
                if (g != 0) p_exit(0);
                a_close(link[1]);
                char e;
                a_read(link[0], &e, 1); /* EOF: the parent is on its way out */
                for (int k = 0; k < 1000 && syscall0(SYS_getppid) != 1; k++) syscall0(SYS_sched_yield);
            }
            a_close(alive[0]);
            char x;
            a_read(blk[0], &x, 1); /* blocks until killed: the write end is held by every process of the namespace */
            p_exit(3);
        }
    }
    a_write(report[1], "r", 1);
    return 7;
}

static int subreaper_body(void) {
    i32 rep[2];
    p_pipe(rep);
    i64 sr = p_fork();
    if (sr == 0) {
        syscall2(SYS_prctl, PR_SET_CHILD_SUBREAPER, 1);
        int flag = 0;
        i64 g = p_fork();
        if (g == 0) {
            i32 link[2];
            p_pipe(link);
            i64 gg = p_fork();
            if (gg == 0) {
                a_close(link[1]);
                char e;
                a_read(link[0], &e, 1); /* EOF: our parent is on its way out */
                i64 par = syscall0(SYS_getppid);
                for (int k = 0; k < 1000 && syscall0(SYS_getppid) == par; k++) syscall0(SYS_sched_yield);
                char v = syscall0(SYS_getppid) == 1 ? '1' : 's'; /* 's': a subreaper's pid, not 1 */
                a_write(rep[1], &v, 1);
                p_exit(0);
            }
            p_exit(0);
        }
        i32 st;
        syscall4(SYS_wait4, g, &st, 0, 0);
        char v = 0;
        a_read(rep[0], &v, 1);
        syscall4(SYS_wait4, -1, &st, 0, 0); /* the orphan, now our child */
        syscall2(SYS_prctl, PR_GET_CHILD_SUBREAPER, (i64)&flag);
        p_exit(v == 's' ? (flag == 1 ? 0 : 20) : 21);
    }
    i32 st = -1;
    syscall4(SYS_wait4, sr, &st, 0, 0);
    return st == 0 ? 0 : 1;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    p_pipe(alive);
    p_pipe(report);
    p_pipe(blk);
    i64 init = p_clone(CLONE_NEWPID);
    if (init == 0) {
        a_close(report[0]);
        p_exit(init_exits());
    }
    a_close(report[1]);
    char c = 0;
    expect("init_started_helpers", a_read(report[0], &c, 1), 1);
    int st = p_wait(init);
    expect("init_exit_status_7", st, P_EXITED(7));
    /* Every helper holds the write end of `alive`: a read that ends with
     * end-of-file (0) proves they are all gone. */
    a_close(alive[1]);
    char z;
    expect("all_helpers_gone_eof", a_read(alive[0], &z, 1), 0);
    expect("no_child_left", syscall4(SYS_wait4, -1, &st, 0, 0), -ECHILD);

    i64 sub = p_clone(CLONE_NEWPID);
    if (sub == 0) p_exit(subreaper_body());
    st = p_wait(sub);
    expect("subreaper_in_namespace", st, 0);
    return finish();
}
