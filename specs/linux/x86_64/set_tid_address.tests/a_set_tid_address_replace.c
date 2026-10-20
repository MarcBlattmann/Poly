/* set_tid_address replaces the address given with CLONE_CHILD_CLEARTID:
 * only the latest one is cleared; NULL clears nothing. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 first, second, third, fourth;

static int to_second(void *arg) {
    (void)arg;
    syscall1(SYS_set_tid_address, &second);
    return 0;
}

static int to_null(void *arg) {
    (void)arg;
    syscall1(SYS_set_tid_address, 0);
    third = 1;
    return 0;
}

int test_main(void) {
    first = second = 0x77;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&first, 0, to_second, 0);
    int cleared = a_spin_until(&second, 0, A_SPIN_LIMIT);
    check("latest_address_cleared", cleared, second);
    expect("earlier_address_untouched", first, 0x77);
    fourth = 0x99;
    third = 0;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[1] + A_STACK_SIZE, 0, (i32 *)&fourth, 0, to_null, 0);
    /* The thread unregisters, so nothing tells us when it ends; it makes
     * no system call after setting `third`, so give it time to exit. */
    a_spin_until(&third, 1, A_SPIN_LIMIT);
    a_spin_ticks(300000000UL);
    expect("null_clears_nothing", fourth, 0x99);
    return finish();
}
