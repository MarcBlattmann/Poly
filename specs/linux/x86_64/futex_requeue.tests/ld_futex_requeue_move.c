/* futex_requeue wakes up to nr_wake tasks waiting on waiters[0] and moves
 * up to nr_requeue of the rest to waiters[1] without waking them, longest
 * waiting first, and returns woken + moved. Three tasks wait on A (two
 * with futex_wait, one with the classic FUTEX_WAIT): requeue(1, 1) returns
 * 2 (the first is woken, the second moved to B, the third stays on A); a
 * moved task is then woken by a wake of B, not of A; the woken tasks'
 * calls return 0. requeue(0, many) moves all three to B (returns 3), and
 * one wake of B with a large nr wakes them all.
 * poly-diff: syscalls=off (several tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex 202
#define SYS_futex_wake 454
#define SYS_futex_wait 455
#define SYS_futex_requeue 456
/* linux/futex.h */
#define FUTEX_WAIT 0
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128
#define U32P (FUTEX2_SIZE_U32 | FUTEX2_PRIVATE)

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

static i64 queued(u32 *p) { return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 0x7fffffff, p, *p); }

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
    if (i == 1)
        result[i] = SC6(SYS_futex, A, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 7, 0, 0, 0);
    else
        result[i] = SC6(SYS_futex_wait, A, 7, 0xffffffff, U32P, 0, 0);
    return 0;
}

static void start_three(void) {
    for (long i = 0; i < 3; i++) {
        result[i] = 99;
        pids[i] = a_spawn((int)i, &done[i], waiter, (void *)i);
        wait_queued(A, i + 1);
    }
}

static int ended(int i) { return a_spin_until(&done[i], 0, A_SPIN_LIMIT); }

static void reap_all(void) {
    for (int i = 0; i < 3; i++) {
        ended(i);
        syscall4(SYS_wait4, pids[i], 0, 0, 0);
    }
}

int test_main(void) {
    a_block_sigchld();
    *A = 7;
    *B = 0;
    v[0] = (struct futex_waitv){7, (u64)A, U32P, 0};
    v[1] = (struct futex_waitv){0, (u64)B, U32P, 0};

    start_three();
    expect("three_on_a", queued(A), 3);
    expect("requeue_1_1", syscall4(SYS_futex_requeue, v, 0, 1, 1), 2);
    check("first_woken", ended(0), done[0]);
    expect("first_result_0", result[0], 0);
    expect("one_left_on_a", queued(A), 1);
    expect("one_on_b", queued(B), 1);
    expect("second_still_sleeping", done[1], -1);
    expect("third_still_sleeping", done[2], -1);
    expect("wake_b", syscall4(SYS_futex_wake, B, 0xffffffff, 10, U32P), 1);
    check("second_woken_from_b", ended(1), done[1]);
    expect("second_result_0", result[1], 0);
    expect("third_still_sleeping_after_b", done[2], -1);
    expect("wake_a", syscall4(SYS_futex_wake, A, 0xffffffff, 10, U32P), 1);
    check("third_woken", ended(2), done[2]);
    expect("third_result_0", result[2], 0);
    reap_all();

    start_three();
    expect("requeue_0_all", syscall4(SYS_futex_requeue, v, 0, 0, 100), 3);
    expect("none_on_a", queued(A), 0);
    expect("three_on_b", queued(B), 3);
    expect("wake_a_finds_none", syscall4(SYS_futex_wake, A, 0xffffffff, 10, U32P), 0);
    expect("wake_b_all", syscall4(SYS_futex_wake, B, 0xffffffff, 0x7fffffff, U32P), 3);
    reap_all();
    expect("results_0", result[0] + result[1] + result[2], 0);

    start_three();
    expect("requeue_2_0_wakes_two", syscall4(SYS_futex_requeue, v, 0, 2, 0), 2);
    check("oldest_woken", ended(0), done[0]);
    check("next_woken", ended(1), done[1]);
    expect("one_still_on_a", queued(A), 1);
    expect("requeue_5_5_last", syscall4(SYS_futex_requeue, v, 0, 5, 5), 1);
    reap_all();
    expect("nobody_left", queued(A) + queued(B), 0);
    return finish();
}
