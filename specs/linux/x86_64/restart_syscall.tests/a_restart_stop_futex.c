/* FUTEX_WAIT stopped by SIGSTOP and continued by SIGCONT. A stopped
 * waiter is not waiting: FUTEX_WAKE while it is stopped wakes nobody
 * (returns 0). After the continue the wait is restarted on the same word
 * and value: timed waits through restart_syscall against the original
 * deadline (-ETIMEDOUT at the original end, the stopped time counts), a
 * word changed meanwhile gives -EAGAIN right after the continue, and an
 * untimed wait goes on waiting until a later wake (then 0).
 * poly-diff: syscalls=off (two processes, timing)
 */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_clock_nanosleep 230
#define TIMER_ABSTIME 1
#define WUNTRACED 2

/* Shared with the child: [0] ready, [1] result, [2] elapsed ms, [3] rem ms. */
static volatile i64 *sh;
static __attribute__((unused)) volatile unsigned *word;

static i64 ms_since(i64 t0) { return (now_ns() - t0) / 1000000; }

static void sleep_until(i64 t0, i64 ms) {
    i64 left = ms - ms_since(t0);
    if (left > 0) sleep_ms(left);
}

static __attribute__((unused)) void cat(char *out, const char *a, const char *b) {
    while (*a) *out++ = *a++;
    while (*b) *out++ = *b++;
    *out = 0;
}

/* Wait up to 2 s for `pid` to report a stop; 1 if it did. */
static int wait_stopped(i64 pid) {
    for (int i = 0; i < 2000; i++) {
        int st = 0;
        i64 w = syscall4(SYS_wait4, pid, &st, WUNTRACED | WNOHANG, 0);
        if (w == pid) return (st & 0xff) == 0x7f;
        if (w < 0) return 0;
        sleep_ms(1);
    }
    return 0;
}

/* Fork a child that runs body() (which sets sh[1] and sh[3]) and times
 * it into sh[2]. Once it is about to block, stop it with `sig` at
 * stop_ms, run `while_stopped`, continue it at cont_ms, and reap it. */
static int stop_cont(i64 (*body)(void), int sig, i64 stop_ms, void (*while_stopped)(void), i64 cont_ms) {
    sh[0] = 0;
    sh[1] = 999;
    sh[2] = -1;
    sh[3] = -1;
    i64 pid = fork();
    if (pid == 0) {
        /* A process group of its own, so SIGTSTP/SIGTTIN can stop it
         * (they are dropped for orphaned process groups). */
        syscall2(SYS_setpgid, 0, 0);
        i64 t0 = now_ns();
        sh[0] = 1;
        i64 r = body();
        sh[2] = ms_since(t0);
        sh[1] = r;
        syscall1(SYS_exit_group, 0);
    }
    if (pid < 0) return 0;
    while (!sh[0]) sleep_ms(1);
    i64 t0 = now_ns();
    sleep_until(t0, stop_ms);
    syscall2(SYS_kill, pid, sig);
    if (!wait_stopped(pid)) {
        /* Never stopped: end it rather than wait for a call that may
         * never return. */
        syscall2(SYS_kill, pid, SIGKILL);
        wait_status(pid);
        return 0;
    }
    if (while_stopped) while_stopped();
    sleep_until(t0, cont_ms);
    syscall2(SYS_kill, pid, SIGCONT);
    return wait_exit(pid) == 0;
}

static i64 woken;

static i64 wait_timed(void) {
    struct k_timespec rel = { 0, 400000000 };
    return futex(word, FUTEX_WAIT, 0, &rel, 0, 0);
}
static i64 wait_forever(void) { return futex(word, FUTEX_WAIT, 0, 0, 0, 0); }
static void wake(void) { woken = futex(word, FUTEX_WAKE, 1, 0, 0, 0); }
static void change(void) { *word = 1; }

int test_main(void) {
    sh = shared_page(4096);
    word = (volatile unsigned *)(sh + 64);

    *word = 0;
    woken = 99;
    check("timed_cycle", stop_cont(wait_timed, SIGSTOP, 100, wake, 300), 0);
    check("timed_wake_while_stopped_wakes_none", woken == 0, woken);
    check("timed_etimedout", sh[1] == -ETIMEDOUT, sh[1]);
    check("timed_ends_at_original_deadline", sh[2] >= 399 && sh[2] < 560, sh[2]);

    *word = 0;
    check("changed_cycle", stop_cont(wait_timed, SIGSTOP, 100, change, 200), 0);
    check("changed_eagain", sh[1] == -EAGAIN, sh[1]);
    check("changed_after_continue", sh[2] >= 195 && sh[2] < 399, sh[2]);

    /* Untimed: stopped, woken in vain, continued; then woken for real. */
    *word = 0;
    woken = 99;
    sh[0] = 0;
    sh[1] = 999;
    i64 pid = fork();
    if (pid == 0) {
        sh[0] = 1;
        sh[1] = wait_forever();
        syscall1(SYS_exit_group, 0);
    }
    while (!sh[0]) sleep_ms(1);
    sleep_ms(100);
    syscall2(SYS_kill, pid, SIGSTOP);
    int stopped = wait_stopped(pid);
    check("untimed_stopped", stopped, 0);
    wake();
    check("untimed_wake_while_stopped_wakes_none", woken == 0, woken);
    syscall2(SYS_kill, pid, SIGCONT);
    sleep_ms(200);
    check("untimed_still_waiting", sh[1] == 999, sh[1]);
    i64 n = 0;
    for (int i = 0; i < 200 && n == 0; i++) {
        n = futex(word, FUTEX_WAKE, 1, 0, 0, 0);
        if (n == 0) sleep_ms(5);
    }
    check("untimed_later_wake_wakes_one", n == 1, n);
    if (n != 1) syscall2(SYS_kill, pid, SIGKILL);
    check("untimed_child_exit", wait_exit(pid) == 0, 0);
    check("untimed_returns_0", sh[1] == 0, sh[1]);
    return finish();
}
