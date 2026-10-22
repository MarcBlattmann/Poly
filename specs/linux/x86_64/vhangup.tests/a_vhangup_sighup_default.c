/* A refused (EPERM) vhangup sends nothing: a session leader with a
 * controlling terminal and SIGHUP at its default action (not blocked)
 * goes on running.
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

/* Give up root, if the test runs as root, so the call is unprivileged
 * whatever the user: uid and gid 65534, no groups, no capabilities. */
static int become_unprivileged(void) {
    if (syscall0(SYS_geteuid) != 0) return 1;
    syscall2(SYS_setgroups, 0, 0);
    syscall3(SYS_setresgid, 65534, 65534, 65534);
    syscall3(SYS_setresuid, 65534, 65534, 65534);
    return syscall0(SYS_geteuid) == 65534;
}

/* Open a pty pair; *slave gets the slave side (not yet anyone's
 * controlling terminal). Returns the master, or -errno. */
static int open_pty(int *slave) {
    int m = (int)syscall4(SYS_openat, AT_FDCWD, "/dev/ptmx", O_RDWR | O_NOCTTY, 0);
    if (m < 0) return m;
    int zero = 0;
    syscall3(SYS_ioctl, m, TIOCSPTLCK, &zero);
    *slave = (int)syscall3(SYS_ioctl, m, TIOCGPTPEER, O_RDWR | O_NOCTTY);
    return m;
}

static int body(void) {
    check("unprivileged", become_unprivileged(), 0);
    sig_action(SIGHUP, SIG_DFL, 0, 0);
    sig_mask(SIG_UNBLOCK, 1UL << (SIGHUP - 1), 0);
    int s = -1;
    int m = open_pty(&s);
    check("pty", m >= 0 && s >= 0, m);
    expect("tiocsctty", syscall3(SYS_ioctl, s, TIOCSCTTY, 0), 0);
    expect("vhangup", syscall0(SYS_vhangup), -EPERM);
    expect("vhangup_again", syscall0(SYS_vhangup), -EPERM);
    check("still_running", 1, 0);
    return finish();
}

int test_main(void) { return in_session(body); }
