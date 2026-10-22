/* vhangup as root in a session that has no controlling terminal: 0, with
 * nothing to hang up; no signal; again 0.
 * poly-diff: needs=root (CAP_SYS_TTY_CONFIG)
 */
#include "poly_ipc.h"

#define SYS_vhangup 153
#define CLONE_VFORK 0x00004000

static void expect(const char *name, i64 got, i64 want) { check(name, got == want, got); }

/* fork with the parent suspended until the child has ended. */
static i64 fork_sync(void) { return syscall2(SYS_clone, CLONE_VFORK | SIGCHLD, 0); }

/* Run `body` in a child that leads a new session (no controlling
 * terminal); its output and exit status are the test's. */
static int in_session(int (*body)(void)) {
    u64 chld = 1UL << (SIGCHLD - 1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &chld, 0, 8);
    i64 pid = fork_sync();
    if (pid == 0) exit_group(syscall0(SYS_setsid) > 0 ? body() : 99);
    int code = wait_exit(pid);
    return code < 0 ? 1 : code;
}

#define HUPCONT ((1UL << (SIGHUP - 1)) | (1UL << (SIGCONT - 1)))

static int body(void) {
    sig_mask(SIG_BLOCK, HUPCONT, 0);
    i64 tty = syscall4(SYS_openat, AT_FDCWD, "/dev/tty", O_RDWR | O_NOCTTY, 0);
    expect("no_controlling_terminal", tty, -ENXIO);
    expect("vhangup", syscall0(SYS_vhangup), 0);
    expect("vhangup_junk_args", syscall6(SYS_vhangup, -1, 7, 7, 7, 7, 7), 0);
    expect("no_sighup_sigcont", (i64)(sig_pending() & HUPCONT), 0);
    expect("stdout_still_writes", write(1, "", 0), 0);
    return finish();
}

int test_main(void) { return in_session(body); }
