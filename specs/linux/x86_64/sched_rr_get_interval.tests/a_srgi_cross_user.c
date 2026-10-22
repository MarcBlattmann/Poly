/* sched_rr_get_interval makes no permission check: a process that has
 * dropped to uid/gid 65534 can read the interval of its root-owned
 * SCHED_RR parent (100 ms) and of a root-owned SCHED_OTHER process.
 * poly-diff: needs=root (needs a SCHED_RR parent and a uid switch)
 */
#include "a_sys.h"

#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_sched_setscheduler 144
#define SYS_sched_rr_get_interval 148
#define SCHED_RR 2

struct a_ts { i64 sec, nsec; };

int test_main(void) {
    a_block_sigchld();
    i64 top = syscall0(SYS_getpid);
    long pid = a_fork_sync();
    if (pid == 0) {
        i32 prio = 3;
        expect("set_rr", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &prio), 0);
        long g = a_fork_sync();
        if (g == 0) {
            int bad = 0;
            if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) bad |= 8;
            if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) bad |= 8;
            if (syscall0(SYS_getuid) != 65534) bad |= 8;
            struct a_ts p = {-1, -1}, t = {-1, -1};
            if (syscall2(SYS_sched_rr_get_interval, syscall0(SYS_getppid), &p) != 0) bad |= 1;
            else if (p.sec != 0 || p.nsec != 100000000) bad |= 2;
            if (syscall2(SYS_sched_rr_get_interval, top, &t) != 0) bad |= 4;
            else if (t.sec != 0 || t.nsec < 0 || t.nsec >= 1000000000) bad |= 4;
            exit_group(bad);
        }
        i32 st = -1;
        expect("reap_grandchild", syscall4(SYS_wait4, g, &st, 0, 0), g);
        expect("grandchild_ok", st, 0);
        exit_group(failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_ok", st, 0);
    return finish();
}
