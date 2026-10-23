/* A process of another user can read a root process's mask and root can
 * read a mask of another user's process: there is no permission check. A
 * mask set by root on a uid 65534 process shows to both.
 * poly-diff: needs=root (only root can make processes of other users)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setresuid 117
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[2] = {0, 0};
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static volatile i32 done, go, ready;
static volatile u64 saw_parent, saw_self;

static int target(void *arg) {
    i64 parent = (i64)arg;
    if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) {
        ready = 2;
        return 1;
    }
    saw_parent = mask_of(parent);
    ready = 1;
    while (!go) a_relax();
    saw_self = mask_of(0);
    ready = 3;
    while (go != 2) a_relax();
    return 0;
}

int test_main(void) {
    /* Not root: nothing here can be set up, so stop at once. */
    i64 euid = syscall0(SYS_geteuid);
    expect("is_root", euid, 0);
    if (euid != 0) return finish();
    a_block_sigchld();
    u64 start = mask_of(0);
    u64 lo = start & -start;
    u64 v = lo;
    expect("root_set_own", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);

    long t = a_spawn(0, &done, target, (void *)syscall0(SYS_getpid));
    while (ready == 0) a_relax();
    check("target_is_65534", ready == 1, ready);
    if (ready == 1) {
        check("user_reads_root", saw_parent == lo, 0);
        check("root_reads_user", mask_of(t) == lo, 0);
        v = start;
        expect("root_sets_user", syscall3(SYS_sched_setaffinity, t, 8, &v), 0);
        check("root_reads_user_new", mask_of(t) == start, 0);
        go = 1;
        a_spin_until(&ready, 3, A_SPIN_LIMIT);
        check("user_reads_own_new", saw_self == start, 0);
    }
    go = 2;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st = -1;
    expect("reap_target", syscall4(SYS_wait4, t, &st, 0, 0), t);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    return finish();
}
