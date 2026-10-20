/* gettimeofday: bad pointers are EFAULT; the time read from a thread
 * agrees with the main thread's. */
#include "a_sys.h"

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid;
static struct a_timeval in_thread;

static int thread(void *arg) {
    (void)arg;
    syscall2(SYS_gettimeofday, &in_thread, 0);
    return 0;
}

int test_main(void) {
    expect("bad_tv_efault", syscall2(SYS_gettimeofday, (void *)8, 0), -EFAULT);
    expect("bad_tz_efault", syscall2(SYS_gettimeofday, 0, (void *)8), -EFAULT);
    struct a_timeval before, after;
    syscall2(SYS_gettimeofday, &before, 0);
    ctid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, thread, 0);
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    syscall2(SYS_gettimeofday, &after, 0);
    check("thread_time_between", in_thread.tv_sec >= before.tv_sec && in_thread.tv_sec <= after.tv_sec,
          in_thread.tv_sec);
    return finish();
}
