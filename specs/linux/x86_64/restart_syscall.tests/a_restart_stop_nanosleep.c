/* A nanosleep stopped by SIGSTOP and continued by SIGCONT (no handlers)
 * is restarted by the kernel through restart_syscall: to the program it
 * returns 0 at the sleep's ORIGINAL end, so the time spent stopped
 * counts (stopped at 100 ms, continued at 300 ms: done at 400 ms, not
 * 600). Continued after the original end, it returns 0 right away. The
 * stop writes rem (the time left when it stopped) even though the call
 * then succeeds. A NULL rem works the same.
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

static i64 sleep_rem(void) {
    struct k_timespec req = { 0, 400000000 }, rem = { 7, 7 };
    i64 r = syscall2(SYS_nanosleep, &req, &rem);
    sh[3] = rem.tv_sec * 1000 + rem.tv_nsec / 1000000;
    return r;
}

static i64 sleep_null(void) {
    struct k_timespec req = { 0, 400000000 };
    return syscall2(SYS_nanosleep, &req, 0);
}

int test_main(void) {
    sh = shared_page(4096);
    check("cycle_inside", stop_cont(sleep_rem, SIGSTOP, 100, 0, 300), 0);
    check("inside_returns_0", sh[1] == 0, sh[1]);
    check("inside_ends_at_original_deadline", sh[2] >= 399 && sh[2] < 560, sh[2]);
    check("inside_rem_written_at_stop", sh[3] >= 200 && sh[3] <= 400, sh[3]);

    check("cycle_past_end", stop_cont(sleep_rem, SIGSTOP, 100, 0, 700), 0);
    check("past_end_returns_0", sh[1] == 0, sh[1]);
    check("past_end_returns_at_continue", sh[2] >= 695 && sh[2] < 900, sh[2]);

    check("cycle_null_rem", stop_cont(sleep_null, SIGSTOP, 100, 0, 300), 0);
    check("null_rem_returns_0", sh[1] == 0, sh[1]);
    check("null_rem_ends_at_original_deadline", sh[2] >= 399 && sh[2] < 560, sh[2]);
    return finish();
}
