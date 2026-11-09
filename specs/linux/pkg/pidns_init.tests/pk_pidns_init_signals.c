/* Signals and the first process (pid 1) of a PID namespace, in the parts a
 * ptrace tracer does not change (a traced pid 1 receives the signals it
 * would otherwise ignore: those cases are in the spec sheet only). A
 * SIGKILL sent from the ancestor namespace kills pid 1: the parent's wait
 * status is "killed by 9". With a handler installed, SIGUSR1 sent from the
 * ancestor namespace is delivered (the blocked read returns EINTR and the
 * handler ran). The processes below pid 1 have no such protection: pid 1
 * kills a child with SIGTERM or with SIGKILL (the wait status shows the
 * signal), and when pid 1 is killed from outside while it has a child,
 * the child is gone with it (the parent has no further child to wait for). A child of pid 1 is reparented to pid 1
 * when its own parent exits: a grandchild sees getppid() 1 after its
 * parent (pid 2) exited.
 * poly-diff: needs=root (a new PID namespace needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes)
 */
#include "p_ns.h"

static i32 hold[2], ready[2];
static volatile int got_usr1;

static void on_usr1(int sig) {
    (void)sig;
    got_usr1 = 1;
}

/* The init processes print nothing: the parent reports. */
static int wait_for_release(void) {
    char c;
    p_send_byte(ready[1], 'r');
    i64 r = a_read(hold[0], &c, 1);
    return r == 1 ? 0 : (int)(r == -EINTR ? 40 : 41);
}

static int b_plain(void) {
    int bad = 0;
    i32 st = -1;
    i32 pipefd[2];
    p_pipe(pipefd);
    i64 y = p_fork();
    if (y == 0) {
        char c;
        a_read(pipefd[0], &c, 1);
        p_exit(0);
    }
    bad |= syscall2(SYS_kill, y, SIGTERM) != 0;
    syscall4(SYS_wait4, y, &st, 0, 0);
    bad |= !(A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGTERM);
    i64 z = p_fork();
    if (z == 0) {
        char c;
        a_read(pipefd[0], &c, 1);
        p_exit(0);
    }
    bad |= syscall2(SYS_kill, z, SIGKILL) != 0;
    syscall4(SYS_wait4, z, &st, 0, 0);
    bad |= !(A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL);
    int r = wait_for_release();
    return bad ? 50 : r;
}

/* pid 1 forks a child (pid 2) that forks a grandchild and exits; the
 * grandchild reports its new parent through a pipe. */
static int b_reparent(void) {
    i32 rep[2];
    p_pipe(rep);
    i64 c = p_fork();
    if (c == 0) {
        i64 g = p_fork();
        if (g == 0) {
            /* wait until our parent is gone: getppid becomes 1 */
            for (int i = 0; i < 5000 && syscall0(SYS_getppid) != 1; i++) {
                i64 ts[2] = {0, 1000000};
                syscall2(SYS_nanosleep, ts, 0);
            }
            char v = (char)('0' + syscall0(SYS_getppid));
            a_write(rep[1], &v, 1);
            p_exit(0);
        }
        p_exit(0);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    char v = 0;
    a_read(rep[0], &v, 1);
    int bad = st != 0 || v != '1';
    syscall4(SYS_wait4, -1, &st, 0, 0); /* the reparented grandchild */
    int r = wait_for_release();
    return bad ? 50 : r;
}

static int b_handler(void) {
    p_handler(SIGUSR1, on_usr1);
    int r = wait_for_release();
    return (r == 40 && got_usr1) ? 40 : 60;
}

/* pid 1 with a blocked child; killed from outside. */
static int b_with_child(void) {
    i64 c = p_fork();
    if (c == 0) {
        char x;
        a_read(hold[0], &x, 1);
        p_exit(0);
    }
    return wait_for_release();
}

/* Wait until process `pid` (seen from the creator's /proc) sleeps. */
static void wait_sleeping(i64 pid) {
    char path[48], text[256];
    p_proc_path(path, pid, "stat");
    for (int i = 0; i < 5000; i++) {
        i64 n = p_read_file(path, text, sizeof text);
        if (n > 0) {
            const char *q = text;
            while (*q && *q != ')') q++;
            if (q[0] == ')' && q[2] == 'S') return;
        }
        i64 ts[2] = {0, 1000000};
        syscall2(SYS_nanosleep, ts, 0);
    }
}

static i64 start(int (*body)(void)) {
    p_pipe(hold);
    p_pipe(ready);
    i64 pid = p_clone(CLONE_NEWPID);
    if (pid == 0) {
        a_close(hold[1]);
        a_close(ready[0]);
        p_exit(body());
    }
    a_close(hold[0]);
    a_close(ready[1]);
    char c = 0;
    a_read(ready[0], &c, 1);
    return pid;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i32 st;

    i64 p = start(b_plain);
    p_send_byte(hold[1], 'x');
    st = p_wait(p);
    expect("init_kills_children", st, 0);

    p = start(b_reparent);
    p_send_byte(hold[1], 'x');
    st = p_wait(p);
    expect("orphan_reparented_to_init", st, 0);

    p = start(b_plain);
    expect("sigkill_from_outside", syscall2(SYS_kill, p, SIGKILL), 0);
    st = p_wait(p);
    check("killed_by_9", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL, st);

    p = start(b_with_child);
    expect("sigkill_init_with_child", syscall2(SYS_kill, p, SIGKILL), 0);
    st = p_wait(p);
    check("killed_with_child_by_9", A_WIFSIGNALED(st) && A_WTERMSIG(st) == SIGKILL, st);
    expect("no_child_left_to_wait_for", syscall4(SYS_wait4, -1, &st, 0, 0), -ECHILD);

    p = start(b_handler);
    wait_sleeping(p);
    expect("sigusr1_with_handler", syscall2(SYS_kill, p, SIGUSR1), 0);
    st = p_wait(p);
    expect("handler_ran_read_interrupted", st, P_EXITED(40));
    return finish();
}
