/* A signal with a handler interrupts futex_wait. Without SA_RESTART the
 * call returns EINTR (4) after the handler ran. With SA_RESTART it is
 * restarted after the handler and goes back to sleep (with or without a
 * deadline, which is absolute and so unchanged by the restart), and a
 * later futex_wake makes it return 0.
 * poly-diff: syscalls=off (two tasks run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))
#define SYS_futex 202
#define SYS_futex_wake 454
#define SYS_futex_wait 455
/* linux/futex.h */
#define FUTEX_CMP_REQUEUE 4
#define FUTEX_PRIVATE_FLAG 128
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128
/* asm-generic/signal-defs.h */
#define SA_RESTART 0x10000000

void ld_restorer(void);
__asm__(".text\n"
        ".global ld_restorer\n"
        "ld_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static u32 word[16] __attribute__((aligned(64)));
static volatile i32 handled;
static volatile i32 helper_done;
static i64 main_pid;
static int restart_case;

static void on_usr1(int sig) {
    (void)sig;
    handled++;
}

static i64 queued(void) {
    return SC6(SYS_futex, word, FUTEX_CMP_REQUEUE | FUTEX_PRIVATE_FLAG, 0, 0x7fffffff, word, 4);
}

static int wait_queued(i64 n) {
    u64 start = a_rdtsc();
    for (;;) {
        if (queued() == n) return 1;
        if (a_rdtsc() - start > A_SPIN_LIMIT) return 0;
        syscall0(SYS_sched_yield);
    }
}

/* Waits until the main task sleeps, signals it; in the restart case waits
 * until it sleeps again after the handler, then wakes it. */
static int helper(void *arg) {
    (void)arg;
    i32 before = handled;
    if (!wait_queued(1)) return 1;
    syscall2(SYS_kill, main_pid, SIGUSR1);
    if (restart_case) {
        if (!a_spin_until(&handled, before + 1, A_SPIN_LIMIT)) return 2;
        if (!wait_queued(1)) return 3;
        syscall4(SYS_futex_wake, word, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE);
    }
    return 0;
}

static i64 round(int restart, int timed) {
    struct a_sigaction sa = {(u64)on_usr1, SA_RESTORER | (restart ? SA_RESTART : 0), (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGUSR1, &sa, 0, 8);
    restart_case = restart;
    struct a_timespec deadline;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &deadline);
    deadline.tv_sec += 20;
    i64 pid = a_spawn(0, &helper_done, helper, 0);
    i64 r = SC6(SYS_futex_wait, word, 4, 0xffffffff, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE, timed ? &deadline : 0,
                CLOCK_MONOTONIC);
    a_spin_until(&helper_done, 0, A_SPIN_LIMIT);
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    return status == 0 ? r : 1000 + status;
}

int test_main(void) {
    a_block_sigchld();
    main_pid = syscall0(SYS_getpid);
    word[0] = 4;
    expect("no_restart_eintr", round(0, 0), -EINTR);
    expect("no_restart_handler_ran", handled, 1);
    expect("no_restart_timed_eintr", round(0, 1), -EINTR);
    expect("no_restart_timed_handler_ran", handled, 2);
    expect("restart_then_woken", round(1, 0), 0);
    expect("restart_handler_ran", handled, 3);
    expect("restart_timed_then_woken", round(1, 1), 0);
    expect("restart_timed_handler_ran", handled, 4);
    return finish();
}
