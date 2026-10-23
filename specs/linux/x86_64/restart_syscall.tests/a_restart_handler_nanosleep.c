/* A nanosleep interrupted by a handled signal leaves something to restart
 * while the handler runs: restart_syscall called in the handler resumes
 * that sleep and returns 0 at the sleep's ORIGINAL end (400 ms after it
 * began, not 400 ms after the restart). Calling it again in the same
 * handler (deadline passed) returns 0 at once. The interrupted nanosleep
 * still returns -EINTR (also with SA_RESTART), its rem holding the time
 * that was left at the interruption; after the handler has returned,
 * restart_syscall is -EINTR again. A nanosleep made inside the handler
 * replaces what there was to restart: restart_syscall after it is -EINTR.
 */
#include "poly_test.h"
#include "poly_ipc.h"

#define SYS_restart_syscall 219

static volatile int mode;
static volatile i64 h_ret, h_ret2, h_end;

static void on_alrm(int sig) {
    (void)sig;
    if (mode == 1) {
        struct k_timespec t = { 0, 5000000 };
        syscall2(SYS_nanosleep, &t, 0);
    }
    h_ret = syscall0(SYS_restart_syscall);
    h_end = now_ns();
    h_ret2 = syscall0(SYS_restart_syscall);
}

static void cat(char *out, const char *a, const char *b) {
    while (*a) *out++ = *a++;
    while (*b) *out++ = *b++;
    *out = 0;
}

static void arm_ms(long ms) {
    struct { long isec, iusec, sec, usec; } v = { 0, 0, ms / 1000, (ms % 1000) * 1000 };
    syscall3(SYS_setitimer, 0, &v, 0);
}

static void run(const char *tag, u64 flags) {
    sig_handler(SIGALRM, on_alrm, flags);
    mode = 0;
    h_ret = h_ret2 = 99;
    struct k_timespec req = { 0, 400000000 }, rem = { 7, 7 };
    arm_ms(100);
    i64 t0 = now_ns();
    i64 r = syscall2(SYS_nanosleep, &req, &rem);
    i64 in_handler_ms = (h_end - t0) / 1000000;
    i64 rem_ms = rem.tv_sec * 1000 + rem.tv_nsec / 1000000;
    char name[64];
    cat(name, tag, "_nanosleep_eintr"); check(name, r == -EINTR, r);
    cat(name, tag, "_restart_in_handler_0"); check(name, h_ret == 0, h_ret);
    cat(name, tag, "_restart_ends_at_original_deadline"); check(name, in_handler_ms >= 399 && in_handler_ms < 560, in_handler_ms);
    cat(name, tag, "_second_restart_0"); check(name, h_ret2 == 0, h_ret2);
    cat(name, tag, "_rem_left_at_interruption"); check(name, rem.tv_sec == 0 && rem_ms > 200 && rem_ms <= 300, rem_ms);
    r = syscall0(SYS_restart_syscall);
    cat(name, tag, "_after_handler_eintr"); check(name, r == -EINTR, r);
}

int test_main(void) {
    run("plain", 0);
    run("sa_restart", SA_RESTART);

    sig_handler(SIGALRM, on_alrm, 0);
    mode = 1;
    struct k_timespec req = { 0, 400000000 };
    arm_ms(100);
    i64 r = syscall2(SYS_nanosleep, &req, 0);
    check("nanosleep_null_rem_eintr", r == -EINTR, r);
    check("restart_after_nanosleep_in_handler_eintr", h_ret == -EINTR, h_ret);
    return finish();
}
