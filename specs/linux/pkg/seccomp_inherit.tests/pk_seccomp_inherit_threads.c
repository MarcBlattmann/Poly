/* Filters belong to a thread. A filter a thread installs is inherited by
 * threads it creates afterwards, but not applied to threads that already
 * exist, unless SECCOMP_FILTER_FLAG_TSYNC is given: then every other
 * thread of the process gets the filter at the same time. TSYNC fails
 * when another thread has a filter that the caller's filters do not
 * contain (it installed one of its own): the call then installs nothing
 * and returns the thread id of such a thread (a positive number, not
 * -1/-errno).
 *
 * Steps (in a child process; threads share memory, so they report through
 * plain variables and are driven by a command word; the waits are spins,
 * no system calls):
 *   1. thread B exists; M (the main thread) installs F (sched_yield -> ERRNO 5):
 *      B is not affected, M is; a thread C created afterwards is;
 *   2. M installs F2 (getuid -> ERRNO 6) with TSYNC: B now has both;
 *   3. B installs its own filter G (getgid -> ERRNO 7); a TSYNC by M with a
 *      further filter returns B's thread id and installs nothing on M or B.
 * poly-diff: syscalls=off (threads)
 */
#include "p_seccomp.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 cmd, ack;
static volatile i64 res;
static volatile i32 b_tid;

static struct pk_insn f_yield[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_sched_yield, 0, 1), PK_ERR(5), PK_ALLOW};
static struct pk_insn f_uid[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getuid, 0, 1), PK_ERR(6), PK_ALLOW};
static struct pk_insn f_gid[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getgid, 0, 1), PK_ERR(7), PK_ALLOW};
static struct pk_insn f_pid[] = {PK_LDABS(SD_NR), PK_JEQ(SYS_getpid, 0, 1), PK_ERR(8), PK_ALLOW};

/* worker thread: runs commands from M */
static int worker(void *arg) {
    (void)arg;
    b_tid = (i32)syscall0(SYS_gettid);
    for (;;) {
        while (!cmd) a_relax();
        i32 c = cmd;
        switch (c) {
        case 1: res = syscall0(SYS_sched_yield); break;
        case 2: res = syscall0(SYS_getuid); break;
        case 3: {
            struct pk_prog p = {4, f_gid};
            res = syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p);
            break;
        }
        case 4: res = syscall0(SYS_getgid); break;
        case 5: return 0;
        }
        cmd = 0;
        ack = c;
    }
}

static i64 ask(i32 c) {
    ack = 0;
    cmd = c;
    a_spin_until(&ack, c, A_SPIN_LIMIT);
    return res;
}

static int thread_c(void *arg) {
    (void)arg;
    res = syscall0(SYS_sched_yield);
    ack = 100;
    return 0;
}

static void body(void) {
    syscall5(SYS_prctl, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    long t = a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, 0, 0, worker, 0);
    check("thread_B_created", t > 0, t);
    while (!b_tid) a_relax();
    check("thread_B_tid_differs", b_tid != (i32)syscall0(SYS_gettid), 0);

    /* 1. a filter on M only */
    struct pk_prog p1 = {4, f_yield};
    expect("M_installs_F", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p1), 0);
    expect("M_filtered", syscall0(SYS_sched_yield), -5);
    expect("existing_thread_B_not_filtered", ask(1), 0);
    ack = 0;
    long c = a_clone(THREAD_FLAGS, a_stacks[1] + A_STACK_SIZE, 0, 0, 0, thread_c, 0);
    check("thread_C_created", c > 0, c);
    a_spin_until(&ack, 100, A_SPIN_LIMIT);
    expect("later_thread_C_inherits_F", res, -5);

    /* 2. TSYNC gives B the filter, and B keeps seeing the new one too */
    struct pk_prog p2 = {4, f_uid};
    expect("M_installs_F2_with_tsync", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, &p2), 0);
    expect("B_now_has_F", ask(1), -5);
    expect("B_now_has_F2", ask(2), -6);
    expect("M_has_F2", syscall0(SYS_getuid), -6);

    /* 3. B installs a filter of its own: TSYNC from M fails with B's tid */
    expect("B_installs_G", ask(3), 0);
    expect("B_filtered_by_G", ask(4), -7);
    expect("M_not_filtered_by_G", syscall0(SYS_getgid) >= 0 ? 1 : 0, 1);
    struct pk_prog p3 = {4, f_pid};
    i64 r = syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, &p3);
    expect("tsync_returns_tid_of_diverging_thread", r, b_tid);
    check("nothing_installed_on_M", syscall0(SYS_getpid) > 0, 0);
    expect("B_unchanged", ask(1), -5);
    /* without TSYNC the same filter installs on M alone */
    expect("M_installs_F3_without_tsync", syscall3(SYS_seccomp, SECCOMP_SET_MODE_FILTER, 0, &p3), 0);
    expect("M_filtered_by_F3", syscall0(SYS_getpid), -8);
    cmd = 5; /* B returns and ends; the child process then exits */
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    pk_in_child(body);
    return finish();
}
