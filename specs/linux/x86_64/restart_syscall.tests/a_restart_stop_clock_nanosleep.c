/* clock_nanosleep stopped by SIGSTOP and continued by SIGCONT returns 0
 * at the original end: relative sleeps on CLOCK_MONOTONIC and
 * CLOCK_REALTIME (restarted through restart_syscall, the stopped time
 * counts; rem gets the time left at the stop) and an absolute
 * (TIMER_ABSTIME) sleep alike.
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

static i64 rel(int clock) {
    struct k_timespec req = { 0, 400000000 }, rem = { 7, 7 };
    i64 r = syscall4(SYS_clock_nanosleep, clock, 0, &req, &rem);
    sh[3] = rem.tv_sec * 1000 + rem.tv_nsec / 1000000;
    return r;
}
static i64 rel_mono(void) { return rel(CLOCK_MONOTONIC); }
static i64 rel_real(void) { return rel(CLOCK_REALTIME); }

static i64 abs_mono(void) {
    struct k_timespec until;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &until);
    until.tv_nsec += 400000000;
    if (until.tv_nsec >= 1000000000) {
        until.tv_nsec -= 1000000000;
        until.tv_sec++;
    }
    return syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, TIMER_ABSTIME, &until, 0);
}

static void one(const char *tag, i64 (*body)(void), int has_rem) {
    char name[64];
    cat(name, tag, "_cycle"); check(name, stop_cont(body, SIGSTOP, 100, 0, 300), 0);
    cat(name, tag, "_returns_0"); check(name, sh[1] == 0, sh[1]);
    cat(name, tag, "_ends_at_original_deadline"); check(name, sh[2] >= 399 && sh[2] < 560, sh[2]);
    if (has_rem) {
        cat(name, tag, "_rem_written_at_stop");
        check(name, sh[3] >= 200 && sh[3] <= 400, sh[3]);
    }
}

int test_main(void) {
    sh = shared_page(4096);
    one("monotonic", rel_mono, 1);
    one("realtime", rel_real, 1);
    one("abstime", abs_mono, 0);
    return finish();
}
