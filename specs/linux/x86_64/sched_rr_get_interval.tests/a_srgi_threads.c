/* sched_rr_get_interval takes thread ids: a second thread can query
 * itself (pid 0 and its tid) and the main thread, and the main thread can
 * query the second thread's tid. All succeed with the same interval. */
#include "a_sys.h"

#define SYS_sched_rr_get_interval 148

struct a_ts { i64 sec, nsec; };

static volatile i32 done;
static volatile i32 release;
static volatile i32 ready;
static volatile i64 main_tid;
static volatile i64 r_self0, r_selftid, r_main;
static struct a_ts t_self0, t_selftid, t_main;

static int thread_fn(void *arg) {
    (void)arg;
    r_self0 = syscall2(SYS_sched_rr_get_interval, 0, &t_self0);
    r_selftid = syscall2(SYS_sched_rr_get_interval, syscall0(SYS_gettid), &t_selftid);
    r_main = syscall2(SYS_sched_rr_get_interval, main_tid, &t_main);
    ready = 1;
    while (!release) a_relax();
    return 0;
}

int test_main(void) {
    struct a_ts mine = {-1, -1}, other = {-1, -1};
    main_tid = syscall0(SYS_gettid);
    expect("main_self", syscall2(SYS_sched_rr_get_interval, 0, &mine), 0);
    done = -1;
    long tid = a_clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM |
                           CLONE_CHILD_CLEARTID,
                       a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&done, 0, thread_fn, 0);
    check("clone", tid > 0, tid);
    while (!ready && done != 0) a_relax();
    expect("thread_tid", syscall2(SYS_sched_rr_get_interval, tid, &other), 0);
    release = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    expect("thread_self0", r_self0, 0);
    expect("thread_selftid", r_selftid, 0);
    expect("thread_main", r_main, 0);
    check("same_self0", t_self0.sec == mine.sec && t_self0.nsec == mine.nsec, t_self0.nsec);
    check("same_selftid", t_selftid.sec == mine.sec && t_selftid.nsec == mine.nsec, t_selftid.nsec);
    check("same_main", t_main.sec == mine.sec && t_main.nsec == mine.nsec, t_main.nsec);
    check("same_tid", other.sec == mine.sec && other.nsec == mine.nsec, other.nsec);
    return finish();
}
