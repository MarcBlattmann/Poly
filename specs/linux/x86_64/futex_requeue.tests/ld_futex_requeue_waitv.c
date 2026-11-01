/* A task sleeping in futex_waitv on [0] A and [1] C can be requeued:
 * futex_requeue from A to B moves its entry for A (returns 1) and leaves
 * its entry for C in place. A wake of B then wakes the task, and its
 * futex_waitv returns 0, the index of the entry that was moved; after it
 * returned it is queued nowhere.
 * poly-diff: syscalls=off (two tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex 202
#define SYS_futex_waitv 449
#define SYS_futex_wake 454
#define SYS_futex_requeue 456
/* linux/futex.h */
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
static u32 *A = &word[0], *B = &word[16], *C = &word[32];
static struct futex_waitv rq[2], wv[2];
static volatile i32 done;
static volatile i64 result;

static i64 queued(u32 *p) { return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 0x7fffffff, p, *p); }

static int waiter(void *arg) {
    (void)arg;
    result = SC6(SYS_futex_waitv, wv, 2, 0, 0, 0, 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    *A = 1;
    *B = 0;
    *C = 2;
    wv[0] = (struct futex_waitv){1, (u64)A, U32P, 0};
    wv[1] = (struct futex_waitv){2, (u64)C, U32P, 0};
    rq[0] = (struct futex_waitv){1, (u64)A, U32P, 0};
    rq[1] = (struct futex_waitv){0, (u64)B, U32P, 0};
    result = 99;
    i64 pid = a_spawn(0, &done, waiter, 0);
    u64 start = a_rdtsc();
    while ((queued(A) != 1 || queued(C) != 1) && a_rdtsc() - start < A_SPIN_LIMIT) syscall0(SYS_sched_yield);
    expect("queued_on_a", queued(A), 1);
    expect("queued_on_c", queued(C), 1);
    expect("requeue_moves_one", syscall4(SYS_futex_requeue, rq, 0, 0, 10), 1);
    expect("none_on_a", queued(A), 0);
    expect("one_on_b", queued(B), 1);
    expect("still_on_c", queued(C), 1);
    expect("still_sleeping", done, -1);
    expect("wake_b", syscall4(SYS_futex_wake, B, 0xffffffff, 1, U32P), 1);
    check("woken", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    expect("returns_moved_entry_index_0", result, 0);
    expect("queued_nowhere", queued(A) + queued(B) + queued(C), 0);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    return finish();
}
