/* Shared futexes between processes (MAP_SHARED memory): three children
 * wait on word A; FUTEX_REQUEUE(A -> B, wake 0) moves them and returns how
 * many it moved, so the parent can tell when all three wait; then
 * FUTEX_WAKE on B wakes exactly as many as asked (2, then 1) and each
 * woken FUTEX_WAIT returns 0. FUTEX_CMP_REQUEUE returns woken + moved;
 * FUTEX_WAKE_OP wakes on the first word and, if the comparison holds, on
 * the second.
 * poly-diff: syscalls=off (four processes; the parent polls until the children wait)
 */
#include "poly_ipc.h"

struct shared {
    unsigned a, b, c, d;
    unsigned woken;
};

static struct shared *sh;

static i64 waiter(volatile unsigned *w) {
    i64 pid = fork();
    if (pid == 0) {
        i64 r = futex(w, FUTEX_WAIT, 0, 0, 0, 0);
        __atomic_add_fetch(&sh->woken, 1, __ATOMIC_SEQ_CST);
        exit_group(r == 0 ? 0 : 1);
    }
    return pid;
}

/* Move waiters from `from` to `to` until `n` have moved in total. */
static void gather(volatile unsigned *from, volatile unsigned *to, int n) {
    int moved = 0;
    while (moved < n) {
        i64 r = futex(from, FUTEX_REQUEUE, 0, (void *)0x7fffffff, to, 0);
        if (r > 0) moved += (int)r;
        else syscall0(SYS_sched_yield);
    }
}

static void wait_woken(unsigned n) {
    while (__atomic_load_n(&sh->woken, __ATOMIC_SEQ_CST) < n) syscall0(SYS_sched_yield);
}

int test_main(void) {
    sh = shared_page(4096);
    check("shared_page", sh != 0, 0);
    i64 kids[3];
    for (int i = 0; i < 3; i++) kids[i] = waiter(&sh->a);
    gather(&sh->a, &sh->b, 3);
    check("all_three_requeued_to_b", 1, 0);
    i64 r = futex(&sh->a, FUTEX_WAKE, 10, 0, 0, 0);
    check("nobody_left_on_a", r == 0, r);
    r = futex(&sh->b, FUTEX_WAKE, 2, 0, 0, 0);
    check("wake_two_of_three", r == 2, r);
    wait_woken(2);
    check("two_woke", sh->woken == 2, sh->woken);
    r = futex(&sh->b, FUTEX_WAKE, 5, 0, 0, 0);
    check("wake_last_one", r == 1, r);
    int ok = 1;
    for (int i = 0; i < 3; i++) ok &= wait_exit(kids[i]) == 0;
    check("woken_waits_return_zero", ok, 0);

    /* CMP_REQUEUE: wake 1, move the rest. */
    sh->woken = 0;
    for (int i = 0; i < 3; i++) kids[i] = waiter(&sh->a);
    gather(&sh->a, &sh->c, 3);
    gather(&sh->c, &sh->a, 3); /* all back on a */
    r = futex(&sh->a, FUTEX_CMP_REQUEUE, 1, (void *)10, &sh->b, 0);
    check("cmp_requeue_counts_woken_and_moved", r == 3, r);
    wait_woken(1);
    r = futex(&sh->b, FUTEX_WAKE, 10, 0, 0, 0);
    check("cmp_requeue_moved_two", r == 2, r);
    ok = 1;
    for (int i = 0; i < 3; i++) ok &= wait_exit(kids[i]) == 0;
    check("cmp_requeue_waiters_return_zero", ok, 0);

    /* CMP_REQUEUE limited to one move: one stays behind on a. */
    sh->woken = 0;
    for (int i = 0; i < 3; i++) kids[i] = waiter(&sh->a);
    gather(&sh->a, &sh->c, 3);
    gather(&sh->c, &sh->a, 3);
    r = futex(&sh->a, FUTEX_CMP_REQUEUE, 1, (void *)1, &sh->b, 0);
    check("cmp_requeue_limited", r == 2, r);
    r = futex(&sh->b, FUTEX_WAKE, 10, 0, 0, 0);
    check("one_moved", r == 1, r);
    r = futex(&sh->a, FUTEX_WAKE, 10, 0, 0, 0);
    check("one_left_behind", r == 1, r);
    for (int i = 0; i < 3; i++) wait_exit(kids[i]);

    /* WAKE_OP: one waiter on a, one on d; the comparison holds. */
    sh->woken = 0;
    kids[0] = waiter(&sh->a);
    kids[1] = waiter(&sh->d);
    gather(&sh->a, &sh->c, 1);
    gather(&sh->c, &sh->a, 1);
    gather(&sh->d, &sh->c, 1);
    gather(&sh->c, &sh->d, 1);
    r = futex(&sh->a, FUTEX_WAKE_OP, 1, (void *)1, &sh->d, FUTEX_OP(FUTEX_OP_SET, 0, FUTEX_OP_CMP_EQ, 0));
    check("wake_op_wakes_both", r == 2, r);
    ok = wait_exit(kids[0]) == 0 && wait_exit(kids[1]) == 0;
    check("wake_op_waiters_return_zero", ok, 0);

    /* WAKE_OP where the comparison fails: only the first word's waiter. */
    sh->woken = 0;
    kids[0] = waiter(&sh->a);
    kids[1] = waiter(&sh->d);
    gather(&sh->a, &sh->c, 1);
    gather(&sh->c, &sh->a, 1);
    gather(&sh->d, &sh->c, 1);
    gather(&sh->c, &sh->d, 1);
    r = futex(&sh->a, FUTEX_WAKE_OP, 1, (void *)1, &sh->d, FUTEX_OP(FUTEX_OP_SET, 0, FUTEX_OP_CMP_NE, 0));
    check("wake_op_failed_cmp_wakes_one", r == 1, r);
    r = futex(&sh->d, FUTEX_WAKE, 1, 0, 0, 0);
    check("second_waiter_still_there", r == 1, r);
    ok = wait_exit(kids[0]) == 0 && wait_exit(kids[1]) == 0;
    check("all_children_done", ok, 0);
    return finish();
}
