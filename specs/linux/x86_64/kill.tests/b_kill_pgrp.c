/* Process-group kill: kill(-pgid, sig) reaches every member of the group
 * and nobody else; kill(0, sig) from a member reaches its whole group; a
 * process can signal its child. Children stand in a group of their own,
 * made with setpgid.
 * poly-diff: syscalls=off (several processes)
 */
#include "poly_ipc.h"

static int ready[2], hold[2];

/* A member: joins group `pgid` (0: its own), says so, then waits. */
static i64 member(i64 pgid, int code) {
    i64 pid = fork();
    if (pid == 0) {
        char c = 'r';
        close(hold[1]);
        sig_action(SIGTERM, SIG_DFL, 0, 0);
        sig_action(SIGUSR1, SIG_DFL, 0, 0);
        if (syscall2(SYS_setpgid, 0, pgid) != 0) exit_group(50);
        write(ready[1], &c, 1);
        read(hold[0], &c, 1); /* until killed, or the parent closes */
        exit_group(code);
    }
    return pid;
}

static volatile int parent_hits;
static void on_term(int sig) {
    (void)sig;
    parent_hits++;
}

int test_main(void) {
    char c;
    syscall2(SYS_pipe2, ready, 0);
    syscall2(SYS_pipe2, hold, 0);
    sig_handler(SIGTERM, on_term, 0);

    i64 a = member(0, 1);
    read(ready[0], &c, 1);
    i64 b = member(a, 2);
    read(ready[0], &c, 1);
    i64 outsider = member(0, 3);
    read(ready[0], &c, 1);
    check("member_in_new_group", syscall1(SYS_getpgid, a) == a, 0);
    check("second_member_joined", syscall1(SYS_getpgid, b) == a, 0);

    i64 r = syscall2(SYS_kill, -a, SIGTERM);
    check("kill_group_returns_zero", r == 0, r);
    int sa = wait_status(a), sb = wait_status(b);
    check("group_leader_killed", W_SIGNALED(sa) && W_TERMSIG(sa) == SIGTERM, sa);
    check("group_member_killed", W_SIGNALED(sb) && W_TERMSIG(sb) == SIGTERM, sb);
    check("parent_not_in_group", parent_hits == 0, parent_hits);
    r = syscall2(SYS_kill, -a, 0);
    check("empty_group_esrch", r == -ESRCH, r);

    /* kill(0, sig) from inside a group of two. */
    i64 x = member(0, 4);
    read(ready[0], &c, 1);
    i64 y = member(x, 5);
    read(ready[0], &c, 1);
    i64 killer = fork();
    if (killer == 0) {
        if (syscall2(SYS_setpgid, 0, x) != 0) exit_group(60);
        sig_action(SIGUSR1, SIG_IGN, 0, 0);
        exit_group(syscall2(SYS_kill, 0, SIGUSR1) == 0 ? 61 : 62);
    }
    check("kill_zero_sender_survives_ignoring", wait_exit(killer) == 61, 0);
    int sx = wait_status(x), sy = wait_status(y);
    check("kill_zero_reaches_leader", W_SIGNALED(sx) && W_TERMSIG(sx) == SIGUSR1, sx);
    check("kill_zero_reaches_member", W_SIGNALED(sy) && W_TERMSIG(sy) == SIGUSR1, sy);

    /* The outsider was in none of these groups. */
    r = syscall2(SYS_kill, outsider, 0);
    check("outsider_alive", r == 0, r);
    close(hold[1]);
    check("outsider_exits_normally", wait_exit(outsider) == 3, 0);
    return finish();
}
