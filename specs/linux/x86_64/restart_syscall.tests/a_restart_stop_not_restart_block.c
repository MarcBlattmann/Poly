/* Calls that are NOT restarted through restart_syscall after a stop:
 * ppoll, select and pselect6 with a 400 ms timeout, stopped at 100 ms
 * and continued at 300 ms, are restarted with the time that was left at
 * the stop (written back into their timeout argument), so the stopped
 * time does not count: they return 0 about 600 ms after they began, not
 * 400; their timeout argument reads 0 afterwards. epoll_wait is not
 * restarted at all: it returns -EINTR right after the continue.
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

static i64 do_ppoll(void) {
    struct k_timespec ts = { 0, 400000000 };
    i64 r = syscall5(SYS_ppoll, 0, 0, &ts, 0, 8);
    sh[3] = ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
    return r;
}

static i64 do_select(void) {
    struct k_timeval tv = { 0, 400000 };
    i64 r = syscall5(SYS_select, 0, 0, 0, 0, &tv);
    sh[3] = tv.tv_sec * 1000 + tv.tv_usec / 1000;
    return r;
}

static i64 do_pselect6(void) {
    struct k_timespec ts = { 0, 400000000 };
    i64 r = syscall6(SYS_pselect6, 0, 0, 0, 0, (i64)&ts, 0);
    sh[3] = ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
    return r;
}

static i64 do_epoll_wait(void) {
    i64 ep = syscall1(SYS_epoll_create1, 0);
    struct k_epoll_event ev;
    return syscall4(SYS_epoll_wait, ep, &ev, 1, 400);
}

static void one(const char *tag, i64 (*body)(void)) {
    char name[64];
    cat(name, tag, "_cycle"); check(name, stop_cont(body, SIGSTOP, 100, 0, 300), 0);
    cat(name, tag, "_returns_0"); check(name, sh[1] == 0, sh[1]);
    cat(name, tag, "_stopped_time_not_counted"); check(name, sh[2] >= 560 && sh[2] < 900, sh[2]);
    cat(name, tag, "_timeout_reads_0"); check(name, sh[3] == 0, sh[3]);
}

int test_main(void) {
    sh = shared_page(4096);
    one("ppoll", do_ppoll);
    one("select", do_select);
    one("pselect6", do_pselect6);
    check("epoll_wait_cycle", stop_cont(do_epoll_wait, SIGSTOP, 100, 0, 300), 0);
    check("epoll_wait_eintr", sh[1] == -EINTR, sh[1]);
    check("epoll_wait_returns_at_continue", sh[2] >= 295 && sh[2] < 399, sh[2]);
    return finish();
}
