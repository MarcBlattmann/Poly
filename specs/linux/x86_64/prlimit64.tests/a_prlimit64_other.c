/* prlimit64 on another process: root can read and set a child's limits,
 * which the child then sees; a pid that does not exist is ESRCH. */
#include "a_sys.h"

static volatile i32 go, done;
static volatile i64 seen_cur;

static int child(void *arg) {
    (void)arg;
    while (!go) a_relax();
    struct a_rlimit l;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &l);
    seen_cur = (i64)l.rlim_cur;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    long pid = a_spawn(0, &done, child, 0);
    struct a_rlimit l, mine;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &mine);
    expect("read_child", syscall4(SYS_prlimit64, pid, RLIMIT_NOFILE, 0, &l), 0);
    check("child_has_parents_limit", l.rlim_cur == mine.rlim_cur && l.rlim_max == mine.rlim_max, (i64)l.rlim_cur);
    l.rlim_cur = 33;
    expect("set_child", syscall4(SYS_prlimit64, pid, RLIMIT_NOFILE, &l, 0), 0);
    go = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_sees_new_limit", seen_cur, 33);
    struct a_rlimit now;
    syscall2(SYS_getrlimit, RLIMIT_NOFILE, &now);
    expect("own_limit_untouched", now.rlim_cur, mine.rlim_cur);
    expect("no_such_pid_esrch", syscall4(SYS_prlimit64, 0x3ffffff0, RLIMIT_NOFILE, 0, &l), -ESRCH);
    return finish();
}
