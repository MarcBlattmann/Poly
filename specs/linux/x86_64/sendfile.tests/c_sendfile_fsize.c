/* RLIMIT_FSIZE on out_fd: a transfer that would cross the limit stops at
 * it and returns the partial count; one that starts at the limit is EFBIG
 * and raises SIGXFSZ (ignored here; at its default it kills the process).
 * *offset only advances by what was written.
 * poly-diff: syscalls=off (child processes)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sendfile 40
/* asm/signal.h */
#define SIGXFSZ 25

#define F "/tmp/c_sendfile_fsize_in"
#define G "/tmp/c_sendfile_fsize_out"

static i64 sendfile(i64 out, i64 in, i64 *off, u64 n) { return syscall4(SYS_sendfile, out, in, off, n); }

/* Child: limit 6 bytes, two sends of 10; exit code is a bit set of checks. */
static int child(int ignore) {
    if (ignore) {
        struct a_sigaction sa = { SIG_IGN, 0, 0, 0 };
        syscall4(SYS_rt_sigaction, SIGXFSZ, &sa, 0, 8);
    }
    i64 in = a_open(F, O_RDONLY, 0);
    i64 out = a_open(G, O_WRONLY | O_TRUNC, 0);
    struct a_rlimit rl = { 6, 6 };
    if (syscall2(SYS_setrlimit, RLIMIT_FSIZE, &rl) != 0) return 0;
    i64 off = 0;
    int bits = 0;
    if (sendfile(out, in, &off, 10) == 6) bits |= 1;
    if (off == 6) bits |= 2;
    if (sendfile(out, in, &off, 10) == -EFBIG) bits |= 4;
    if (off == 6) bits |= 8;
    if (sendfile(out, in, &off, 0) == 0) bits |= 16;
    return bits;
}

int test_main(void) {
    struct a_stat st;
    int status = 0;
    expect("setup_in", a_make_file(F, "0123456789", 0644), 0);
    expect("setup_out", a_make_file(G, "", 0644), 0);

    i64 pid = syscall0(SYS_fork);
    if (pid == 0) syscall1(SYS_exit, child(1));
    expect("wait_ignoring", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("ignoring_exited", (status & 0x7f) == 0, status);
    expect("partial_6", (status >> 8) & 3, 3);
    expect("then_efbig", (status >> 8) & 12, 12);
    expect("count_0_at_limit", (status >> 8) & 16, 16);
    expect("out_stat", a_stat_path(G, &st), 0);
    expect("out_size_6", st.st_size, 6);

    pid = syscall0(SYS_fork);
    if (pid == 0) syscall1(SYS_exit, child(0));
    expect("wait_default", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("killed_by_sigxfsz", status & 0x7f, SIGXFSZ);
    expect("cleanup_in", a_unlink(F), 0);
    expect("cleanup_out", a_unlink(G), 0);
    return finish();
}
