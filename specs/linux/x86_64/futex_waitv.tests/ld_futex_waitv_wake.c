/* futex_waitv sleeps on every futex of its array at once and returns the
 * index of the entry that was woken. A task waits on [0] a private word,
 * [1] a shared word, [2] another private word: a futex_wake of the shared
 * word returns 1 there and the task's call returns 1; a classic private
 * FUTEX_WAKE of [2] makes it return 2; futex_wake of [0] makes it return
 * 0. A wake of the wrong kind (private for the shared entry) finds
 * nothing. Once woken the task sleeps on none of its futexes any more.
 * poly-diff: syscalls=off (several tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SC5(n, a, b, c, d, e) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), 0)
#define SYS_futex 202
#define SYS_futex_waitv 449
#define SYS_futex_wake 454
/* linux/futex.h */
#define FUTEX_WAKE 1
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128
#define U32 FUTEX2_SIZE_U32
#define PRIV FUTEX2_PRIVATE

struct futex_waitv {
    u64 val;
    u64 uaddr;
    u32 flags;
    u32 reserved;
};

static u32 word[64] __attribute__((aligned(64)));
static struct futex_waitv v[4];
static volatile i32 done;
static volatile i64 result;
static i64 pid;

static i64 queued(u32 *p, int priv) {
    return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE | (priv ? FUTEX_PRIVATE_FLAG : 0), 0, 0x7fffffff, p, *p);
}

static int wait_queued(u32 *p, int priv, i64 n) {
    u64 start = a_rdtsc();
    for (;;) {
        if (queued(p, priv) == n) return 1;
        if (a_rdtsc() - start > A_SPIN_LIMIT) return 0;
        syscall0(SYS_sched_yield);
    }
}

static int waiter(void *arg) {
    (void)arg;
    result = SC5(SYS_futex_waitv, v, 3, 0, 0, 0);
    return 0;
}

static u32 *A = word, *B = word + 16, *C = word + 32;

static int start(void) {
    v[0] = (struct futex_waitv){1, (u64)A, U32 | PRIV, 0};
    v[1] = (struct futex_waitv){2, (u64)B, U32, 0};
    v[2] = (struct futex_waitv){3, (u64)C, U32 | PRIV, 0};
    result = 99;
    pid = a_spawn(0, &done, waiter, 0);
    return wait_queued(A, 1, 1) && wait_queued(B, 0, 1) && wait_queued(C, 1, 1);
}

static i64 end(void) {
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    return result;
}

static i64 still_queued(void) { return queued(A, 1) + queued(B, 0) + queued(C, 1); }

int test_main(void) {
    a_block_sigchld();
    *A = 1;
    *B = 2;
    *C = 3;

    check("b_queued_on_all_three", start(), 0);
    expect("b_private_wake_misses", syscall4(SYS_futex_wake, B, 0xffffffff, 1, U32 | PRIV), 0);
    expect("b_shared_wake", syscall4(SYS_futex_wake, B, 0xffffffff, 1, U32), 1);
    expect("b_returns_index_1", end(), 1);
    expect("b_queued_nowhere", still_queued(), 0);

    check("c_queued_on_all_three", start(), 0);
    expect("c_classic_wake", SC6(SYS_futex, C, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, 1, 0, 0, 0), 1);
    expect("c_returns_index_2", end(), 2);
    expect("c_queued_nowhere", still_queued(), 0);

    check("a_queued_on_all_three", start(), 0);
    expect("a_wrong_kind_misses", syscall4(SYS_futex_wake, A, 0xffffffff, 1, U32), 0);
    expect("a_wake", syscall4(SYS_futex_wake, A, 0xffffffff, 5, U32 | PRIV), 1);
    expect("a_returns_index_0", end(), 0);
    expect("a_queued_nowhere", still_queued(), 0);
    return finish();
}
