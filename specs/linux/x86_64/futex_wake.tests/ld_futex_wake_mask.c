/* futex_wake's mask selects waiters: a task waiting with futex_wait's
 * mask m is woken only if m & mask != 0. Waiters with masks 1, 2 and 3:
 * a wake with mask 4 wakes none, mask 2 wakes the 2 and the 3 (returns 2),
 * mask 1 then wakes the 1. A classic FUTEX_WAIT waiter has every bit set
 * and is woken by any mask; the classic FUTEX_WAKE_BITSET wakes futex_wait
 * waiters by the same rule.
 * poly-diff: syscalls=off (several tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))

#define SYS_futex 202
#define SYS_futex_wake 454
#define SYS_futex_wait 455
/* linux/futex.h */
#define FUTEX_WAIT 0
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_WAKE_BITSET 10
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128

static u32 word[16] __attribute__((aligned(64)));
static volatile i32 done[4];
static volatile i64 result[4];
static u64 masks[4] = {1, 2, 3, 0};
static i64 pids[4];

static i64 queued(u32 *p, u32 val, int priv) {
    return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE | (priv ? FUTEX_PRIVATE_FLAG : 0), 0, 0x7fffffff, p, val);
}

static int wait_queued(u32 *p, u32 val, int priv, i64 n) {
    u64 start = a_rdtsc();
    for (;;) {
        i64 q = queued(p, val, priv);
        if (q == n) return 1;
        if (q > n || a_rdtsc() - start > A_SPIN_LIMIT) return 0;
        syscall0(SYS_sched_yield);
    }
}

static int waiter(void *arg) {
    long i = (long)arg;
    if (masks[i])
        result[i] = SC6(SYS_futex_wait, word, 5, masks[i], FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0, 0);
    else
        result[i] = SC6(SYS_futex, word, FUTEX_WAIT | FUTEX_PRIVATE_FLAG, 5, 0, 0, 0);
    return 0;
}

static void start(long i) {
    result[i] = 99;
    pids[i] = a_spawn((int)i, &done[i], waiter, (void *)i);
}

static void reap(int i) {
    a_spin_until(&done[i], 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pids[i], 0, 0, 0);
}

int test_main(void) {
    a_block_sigchld();
    word[0] = 5;
    for (long i = 0; i < 3; i++) {
        start(i);
        wait_queued(word, 5, 1, i + 1);
    }
    expect("three_queued", queued(word, 5, 1), 3);
    expect("mask4_wakes_none", syscall4(SYS_futex_wake, word, 4, 10, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 0);
    expect("mask2_wakes_two", syscall4(SYS_futex_wake, word, 2, 10, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 2);
    check("mask2_waiter_done", a_spin_until(&done[1], 0, A_SPIN_LIMIT), done[1]);
    check("mask3_waiter_done", a_spin_until(&done[2], 0, A_SPIN_LIMIT), done[2]);
    expect("mask2_result", result[1], 0);
    expect("mask3_result", result[2], 0);
    expect("mask1_waiter_still_waiting", done[0], -1);
    expect("one_left", queued(word, 5, 1), 1);
    expect("mask1_wakes_it", syscall4(SYS_futex_wake, word, 0x10001, 10, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 1);
    check("mask1_waiter_done", a_spin_until(&done[0], 0, A_SPIN_LIMIT), done[0]);
    expect("mask1_result", result[0], 0);
    for (int i = 0; i < 3; i++) reap(i);

    /* A classic FUTEX_WAIT waiter matches any mask. */
    start(3);
    wait_queued(word, 5, 1, 1);
    expect("classic_woken_by_high_bit",
           syscall4(SYS_futex_wake, word, 0x80000000, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 1);
    reap(3);
    expect("classic_result", result[3], 0);

    /* The classic FUTEX_WAKE_BITSET uses the same rule on futex_wait waiters. */
    start(1);
    wait_queued(word, 5, 1, 1);
    expect("bitset1_wakes_none", SC6(SYS_futex, word, FUTEX_WAKE_BITSET | FUTEX_PRIVATE_FLAG, 1, 0, 0, 1), 0);
    expect("bitset6_wakes_mask2", SC6(SYS_futex, word, FUTEX_WAKE_BITSET | FUTEX_PRIVATE_FLAG, 1, 0, 0, 6), 1);
    reap(1);
    expect("bitset_result", result[1], 0);
    return finish();
}
