/* clock_nanosleep with a relative time, interrupted by a handled signal:
 * restart_syscall in the handler resumes the sleep on the same clock and
 * returns 0 at the sleep's original end, for CLOCK_MONOTONIC,
 * CLOCK_REALTIME and CLOCK_BOOTTIME. An absolute sleep (TIMER_ABSTIME)
 * leaves nothing to restart: restart_syscall in its handler is -EINTR
 * at once. The interrupted call itself returns -EINTR in both cases. */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_restart_syscall 219
#define SYS_clock_nanosleep 230
#define TIMER_ABSTIME 1

static volatile i64 h_ret, h_end;

static void on_alrm(int sig) {
    (void)sig;
    h_ret = syscall0(SYS_restart_syscall);
    h_end = now_ns();
}

static void arm_ms(long ms) {
    struct { long isec, iusec, sec, usec; } v = { 0, 0, ms / 1000, (ms % 1000) * 1000 };
    syscall3(SYS_setitimer, 0, &v, 0);
}

static void cat(char *out, const char *a, const char *b) {
    while (*a) *out++ = *a++;
    while (*b) *out++ = *b++;
    *out = 0;
}

static void relative(const char *tag, int clock) {
    char name[64];
    struct k_timespec req = { 0, 400000000 }, rem = { 7, 7 };
    h_ret = 99;
    arm_ms(100);
    i64 t0 = now_ns();
    i64 r = syscall4(SYS_clock_nanosleep, clock, 0, &req, &rem);
    i64 ms = (h_end - t0) / 1000000;
    i64 rem_ms = rem.tv_sec * 1000 + rem.tv_nsec / 1000000;
    cat(name, tag, "_call_eintr"); check(name, r == -EINTR, r);
    cat(name, tag, "_restart_0"); check(name, h_ret == 0, h_ret);
    cat(name, tag, "_restart_ends_at_original_deadline"); check(name, ms >= 399 && ms < 560, ms);
    cat(name, tag, "_rem_left_at_interruption"); check(name, rem.tv_sec == 0 && rem_ms > 200 && rem_ms <= 300, rem_ms);
}

int test_main(void) {
    sig_handler(SIGALRM, on_alrm, 0);
    relative("monotonic", CLOCK_MONOTONIC);
    relative("realtime", CLOCK_REALTIME);
    relative("boottime", CLOCK_BOOTTIME);

    struct k_timespec until;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &until);
    until.tv_nsec += 400000000;
    if (until.tv_nsec >= 1000000000) {
        until.tv_nsec -= 1000000000;
        until.tv_sec++;
    }
    h_ret = 99;
    arm_ms(100);
    i64 t0 = now_ns();
    i64 r = syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, TIMER_ABSTIME, &until, 0);
    i64 ms = (h_end - t0) / 1000000;
    check("abstime_call_eintr", r == -EINTR, r);
    check("abstime_restart_eintr", h_ret == -EINTR, h_ret);
    check("abstime_restart_returns_at_once", ms < 300, ms);
    return finish();
}
