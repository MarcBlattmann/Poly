/* A task sleeping in futex_wait returns 0 when woken: by futex_wake, or by
 * the classic futex(FUTEX_WAKE) of the same kind (private or shared), also
 * long before its deadline. A wake of the other kind does not find it.
 * Changing the word while a task sleeps does not wake it (only a wake
 * does), and the woken task returns 0 even though the word no longer
 * holds val.
 * poly-diff: syscalls=off (several tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex 202
#define SYS_futex_wake 454
#define SYS_futex_wait 455
/* linux/futex.h */
#define FUTEX_WAKE 1
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128

static u32 word[16] __attribute__((aligned(64)));
static volatile i32 done;
static volatile i64 result;
static u64 wait_flags, wait_mask;
static struct a_timespec deadline;
static int use_deadline;

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
    (void)arg;
    result = SC6(SYS_futex_wait, word, 4, wait_mask, wait_flags, use_deadline ? &deadline : 0, CLOCK_MONOTONIC);
    return 0;
}

static i64 pid;

/* Start one waiter on word[0] (which holds 4) and wait until it sleeps. */
static int start(u64 flags, u64 mask, int timed) {
    word[0] = 4;
    wait_flags = flags;
    wait_mask = mask;
    use_deadline = timed;
    if (timed) {
        syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &deadline);
        deadline.tv_sec += 20;
    }
    result = 99;
    pid = a_spawn(0, &done, waiter, 0);
    return wait_queued(word, 4, (flags & FUTEX2_PRIVATE) != 0, 1);
}

static int finish_waiter(void) {
    int ok = a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    return ok;
}

int test_main(void) {
    a_block_sigchld();

    check("p2_queued", start(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0xffffffff, 1), 0);
    expect("p2_futex_wake", syscall4(SYS_futex_wake, word, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 1);
    check("p2_done", finish_waiter(), 0);
    expect("p2_result_0_before_deadline", result, 0);

    check("s2_queued", start(FUTEX2_SIZE_U32, 0xffffffff, 0), 0);
    expect("s2_private_wake_misses", syscall4(SYS_futex_wake, word, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE),
           0);
    expect("s2_futex_wake", syscall4(SYS_futex_wake, word, 0xffffffff, 1, FUTEX2_SIZE_U32), 1);
    check("s2_done", finish_waiter(), 0);
    expect("s2_result_0", result, 0);

    check("p1_queued", start(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0xffffffff, 0), 0);
    expect("p1_classic_shared_misses", SC6(SYS_futex, word, FUTEX_WAKE, 1, 0, 0, 0), 0);
    expect("p1_classic_private_wake", SC6(SYS_futex, word, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 1, 0, 0, 0), 1);
    check("p1_done", finish_waiter(), 0);
    expect("p1_result_0", result, 0);

    check("s1_queued", start(FUTEX2_SIZE_U32, 0x2, 1), 0);
    expect("s1_classic_private_misses", SC6(SYS_futex, word, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 1, 0, 0, 0), 0);
    expect("s1_classic_shared_wake", SC6(SYS_futex, word, FUTEX_WAKE, 1, 0, 0, 0), 1);
    check("s1_done", finish_waiter(), 0);
    expect("s1_result_0", result, 0);

    /* A store to the word does not wake the sleeper. */
    check("store_queued", start(FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, 0xffffffff, 0), 0);
    word[0] = 5;
    a_spin_ticks(30000000);
    expect("store_still_sleeping", done, -1);
    expect("store_still_queued", queued(word, 5, 1), 1);
    expect("store_wake", syscall4(SYS_futex_wake, word, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 1);
    check("store_done", finish_waiter(), 0);
    expect("store_result_0", result, 0);
    return finish();
}
