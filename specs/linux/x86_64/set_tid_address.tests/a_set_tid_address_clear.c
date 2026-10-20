/* set_tid_address: returns the caller's thread id and registers an
 * address that the kernel sets to 0 when the thread exits (while the
 * memory is shared with others). */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 word, registered;
static volatile i64 ret_in_thread, tid_in_thread;

static int thread(void *arg) {
    (void)arg;
    ret_in_thread = syscall1(SYS_set_tid_address, &word);
    tid_in_thread = syscall0(SYS_gettid);
    registered = 1;
    return 0;
}

int test_main(void) {
    word = 0x5555;
    long t = a_clone(THREAD_FLAGS, a_stacks[0] + A_STACK_SIZE, 0, 0, 0, thread, 0);
    int cleared = a_spin_until(&word, 0, A_SPIN_LIMIT);
    check("thread", t > 0, t);
    expect("registered", registered, 1);
    expect("returns_caller_tid", ret_in_thread, tid_in_thread);
    expect("returns_clone_result", ret_in_thread, t);
    check("word_cleared_at_thread_exit", cleared, word);
    expect("main_returns_pid", syscall1(SYS_set_tid_address, 0), syscall0(SYS_getpid));
    return finish();
}
