/* futex_requeue compares *waiters[0].uaddr with waiters[0].val before
 * doing anything: with two tasks waiting on A, a changed word gives
 * EAGAIN and nobody is woken or moved. requeue(0, 0) returns 0 and changes
 * nothing; requeue(0, 1) moves one task (returns 1); requeuing A onto A
 * itself returns the number of waiters and wakes nobody. Shared futexes
 * (no FUTEX2_PRIVATE) work the same way, and a classic FUTEX_WAKE of the
 * target wakes a task moved there.
 * poly-diff: syscalls=off (several tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex 202
#define SYS_futex_wait 455
#define SYS_futex_requeue 456
/* linux/futex.h */
#define FUTEX_WAKE 1
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128

struct futex_waitv {
    u64 val;
    u64 uaddr;
    u32 flags;
    u32 reserved;
};

static u32 word[64] __attribute__((aligned(64)));
static u32 *A = &word[0], *B = &word[16];
static struct futex_waitv v[2];
static volatile i32 done[4];
static volatile i64 result[4];
static i64 pids[4];
static u64 kind;

static i64 queued(u32 *p) {
    return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE | ((kind & FUTEX2_PRIVATE) ? FUTEX_PRIVATE_FLAG : 0), 0, 0x7fffffff, p,
               *p);
}

static int wait_queued(u32 *p, i64 n) {
    u64 start = a_rdtsc();
    for (;;) {
        if (queued(p) == n) return 1;
        if (a_rdtsc() - start > A_SPIN_LIMIT) return 0;
        syscall0(SYS_sched_yield);
    }
}

static int waiter(void *arg) {
    long i = (long)arg;
    result[i] = SC6(SYS_futex_wait, A, 3, 0xffffffff, kind, 0, 0);
    return 0;
}

static void run(const char *tag, u64 flags) {
    char name[64];
    kind = flags;
    *A = 3;
    *B = 0;
    v[0] = (struct futex_waitv){3, (u64)A, (u32)flags, 0};
    v[1] = (struct futex_waitv){55, (u64)B, (u32)flags, 0};
    for (long i = 0; i < 2; i++) {
        result[i] = 99;
        pids[i] = a_spawn((int)i, &done[i], waiter, (void *)i);
        wait_queued(A, i + 1);
    }
    expect(str_cat2(name, tag, "_two_on_a"), queued(A), 2);
    *A = 4;
    expect(str_cat2(name, tag, "_changed_word_eagain"), syscall4(SYS_futex_requeue, v, 0, 1, 1), -EAGAIN);
    *A = 3;
    expect(str_cat2(name, tag, "_nothing_moved"), queued(A), 2);
    expect(str_cat2(name, tag, "_nobody_woken"), done[0] + done[1], -2);
    expect(str_cat2(name, tag, "_zero_zero"), syscall4(SYS_futex_requeue, v, 0, 0, 0), 0);
    expect(str_cat2(name, tag, "_still_two"), queued(A), 2);
    v[1].uaddr = (u64)A;
    expect(str_cat2(name, tag, "_onto_itself_counts"), syscall4(SYS_futex_requeue, v, 0, 0, 100), 2);
    expect(str_cat2(name, tag, "_onto_itself_wakes_none"), done[0] + done[1], -2);
    v[1].uaddr = (u64)B;
    expect(str_cat2(name, tag, "_move_one"), syscall4(SYS_futex_requeue, v, 0, 0, 1), 1);
    expect(str_cat2(name, tag, "_one_on_a"), queued(A), 1);
    expect(str_cat2(name, tag, "_one_on_b"), queued(B), 1);
    i64 op = FUTEX_WAKE | ((flags & FUTEX2_PRIVATE) ? FUTEX_PRIVATE_FLAG : 0);
    expect(str_cat2(name, tag, "_classic_wake_b"), SC6(SYS_futex, B, op, 1, 0, 0, 0), 1);
    check(str_cat2(name, tag, "_moved_one_woken"), a_spin_until(&done[0], 0, A_SPIN_LIMIT), done[0]);
    expect(str_cat2(name, tag, "_classic_wake_a"), SC6(SYS_futex, A, op, 1, 0, 0, 0), 1);
    check(str_cat2(name, tag, "_other_woken"), a_spin_until(&done[1], 0, A_SPIN_LIMIT), done[1]);
    expect(str_cat2(name, tag, "_results_0"), result[0] + result[1], 0);
    for (int i = 0; i < 2; i++) syscall4(SYS_wait4, pids[i], 0, 0, 0);
}

int test_main(void) {
    a_block_sigchld();
    run("private", FUTEX2_SIZE_U32 | FUTEX2_PRIVATE);
    run("shared", FUTEX2_SIZE_U32);
    return finish();
}
