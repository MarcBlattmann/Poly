/* Controlling terminals (in a child that starts a session): TIOCSCTTY
 * makes the pty slave the controlling terminal; /dev/tty then opens;
 * TIOCGSID reports the session; TIOCGPGRP the foreground group (the
 * child's); TIOCSPGRP to a group of another session is EPERM; a
 * TIOCSWINSZ that changes the size sends SIGWINCH to the foreground group;
 * the interrupt character typed on the master sends SIGINT to it; when the
 * master is closed, the slave reads 0 (hang-up) and SIGHUP is sent to the
 * session leader. TIOCSCTTY without a session of one's own is EPERM.
 * poly-diff: syscalls=off (child processes and signals)
 */
#include "poly_ipc.h"

static volatile int winch, ints, hups;
static void on_winch(int sig) {
    (void)sig;
    winch++;
}
static void on_int(int sig) {
    (void)sig;
    ints++;
}
static void on_hup(int sig) {
    (void)sig;
    hups++;
}

int test_main(void) {
    int m = (int)syscall4(SYS_openat, AT_FDCWD, "/dev/ptmx", O_RDWR | O_NOCTTY, 0);
    int zero = 0;
    syscall3(SYS_ioctl, m, TIOCSPTLCK, &zero);
    int s0 = (int)syscall3(SYS_ioctl, m, TIOCGPTPEER, O_RDWR | O_NOCTTY);
    /* A forked child is never a session leader (whatever the test was
     * started as). */
    i64 nl = fork();
    if (nl == 0) exit_group(syscall3(SYS_ioctl, s0, TIOCSCTTY, 0) == -EPERM ? 0 : 1);
    check("not_session_leader_eperm", wait_exit(nl) == 0, 0);

    int ready[2], go[2];
    syscall2(SYS_pipe2, ready, 0);
    syscall2(SYS_pipe2, go, 0);
    i64 pid = fork();
    if (pid == 0) {
        int code = 0;
        char c = 'r';
        close(go[1]);
        close(m); /* the parent's master is the only one: its close hangs up */
        sig_handler(SIGWINCH, on_winch, 0);
        sig_handler(SIGINT, on_int, 0);
        sig_handler(SIGHUP, on_hup, 0);
        syscall0(SYS_setsid);
        if (syscall3(SYS_ioctl, s0, TIOCSCTTY, 0) != 0) code |= 1;
        i64 tty = syscall4(SYS_openat, AT_FDCWD, "/dev/tty", O_RDWR, 0);
        if (tty < 0) code |= 2;
        int sid = -1, pg = -1;
        syscall3(SYS_ioctl, s0, TIOCGSID, &sid);
        syscall3(SYS_ioctl, s0, TIOCGPGRP, &pg);
        if (sid != getpid() || pg != getpid()) code |= 4;
        int other = (int)syscall0(SYS_getppid);
        if (syscall3(SYS_ioctl, s0, TIOCSPGRP, &other) != -EPERM) code |= 8;
        struct k_winsize w = { 30, 100, 0, 0 };
        syscall3(SYS_ioctl, s0, TIOCSWINSZ, &w);
        if (winch != 1) code |= 16;
        syscall3(SYS_ioctl, s0, TIOCSWINSZ, &w); /* same size: no signal */
        if (winch != 1) code |= 32;
        write(ready[1], &c, 1);
        read(go[0], &c, 1); /* the parent types ^C, then closes the master */
        while (ints == 0) syscall0(SYS_sched_yield);
        char b[8];
        i64 n;
        do n = read(s0, b, sizeof b);
        while (n == -EINTR);
        if (n != 0 && n != -EIO) code |= 64;
        if (hups != 1) code |= 128;
        exit_group(code);
    }
    close(s0);
    char c;
    read(ready[0], &c, 1);
    struct k_termios t;
    int s1 = (int)syscall3(SYS_ioctl, m, TIOCGPTPEER, O_RDWR | O_NOCTTY);
    syscall3(SYS_ioctl, s1, TCGETS, &t);
    close(s1);
    char intr = (char)t.c_cc[VINTR];
    write(m, &intr, 1);
    write(go[1], "g", 1);
    sleep_ms(30);
    close(m);
    int code = wait_exit(pid);
    check("tiocsctty_ok", code >= 0 && !(code & 1), code);
    check("dev_tty_opens", code >= 0 && !(code & 2), code);
    check("sid_and_foreground_group", code >= 0 && !(code & 4), code);
    check("foreign_group_eperm", code >= 0 && !(code & 8), code);
    check("sigwinch_on_size_change", code >= 0 && !(code & 16), code);
    check("no_sigwinch_same_size", code >= 0 && !(code & 32), code);
    check("hangup_read_ends", code >= 0 && !(code & 64), code);
    check("sighup_to_session_leader", code >= 0 && !(code & 128), code);
    return finish();
}
