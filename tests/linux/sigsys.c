/* The disposition of SIGSYS, seen through sigaction(2)'s old action, in a
 * new process, after fork(2), clone(2) with and without CLONE_SIGHAND, and
 * after execve(2).
 *
 * Chromium's seccomp-bpf sandbox asks for SIGSYS's action before it installs
 * its handler and logs "Existing signal handler when trying to install
 * SIGSYS" when it finds anything: sigaction(2) says a signal starts at
 * SIG_DFL with flags 0 and an empty mask, fork(2) and clone(2) copy the
 * dispositions, execve(2) resets the caught ones to SIG_DFL and keeps the
 * ignored ones. A program that put a signal back to SIG_DFL with the flags
 * of the handler it had (a launcher does, before it execs) must not leave
 * those flags to the program it execs.
 *
 * The test runs as itself (no arguments) and, through execve of
 * /proc/self/exe, as "exec-child". Checks the parent makes on a child read
 * the child's exit status.
 */
#include "poly_test.h"

#define SYS_rt_sigaction 13
#define SYS_fork 57
#define SYS_clone 56
#define SYS_execve 59
#define SYS_wait4 61

#define SIGPIPE 13
#define SIGUSR1 10
#define SIGUSR2 12
#define SIGCHLD 17
#define SIGSYS 31

#define SA_SIGINFO 0x00000004UL
#define SA_RESTORER 0x04000000UL
#define SA_ONSTACK 0x08000000UL
#define SA_NODEFER 0x40000000UL

#define SIG_DFL 0UL
#define SIG_IGN 1UL

#define CLONE_VM 0x00000100UL
#define CLONE_SIGHAND 0x00000800UL
#define CLONE_VFORK 0x00004000UL

/* The kernel's struct sigaction on x86-64: handler, flags, restorer, mask. */
struct ksigaction {
    u64 handler;
    u64 flags;
    u64 restorer;
    u64 mask;
};

static i64 get_action(int sig, struct ksigaction *old) {
    return syscall6(SYS_rt_sigaction, sig, 0, (i64)old, 8, 0, 0);
}

static i64 set_action(int sig, const struct ksigaction *act, struct ksigaction *old) {
    return syscall6(SYS_rt_sigaction, sig, (i64)act, (i64)old, 8, 0, 0);
}

/* Never run: only stored. */
static void handler(int sig) { (void)sig; }

static const u64 HANDLER_FLAGS = SA_SIGINFO | SA_ONSTACK | SA_RESTORER | SA_NODEFER;

static int is_default(const struct ksigaction *a) {
    return a->handler == SIG_DFL && a->flags == 0 && a->restorer == 0 && a->mask == 0;
}

static int same_as_installed(const struct ksigaction *a) {
    return a->handler == (u64)handler && a->flags == HANDLER_FLAGS && a->restorer == (u64)handler &&
           a->mask == (1UL << (SIGUSR1 - 1));
}

static int wait_status(i64 pid) {
    int status = -1;
    i64 r = syscall6(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0);
    return r == pid ? status : -1;
}

/* The status of a child that called exit_group(code). */
#define EXITED(code) ((code) << 8)

/* execve(2) of this program as "exec-child": SIGSYS and SIGUSR1 are at the
 * default as a new process has them, SIGPIPE is still ignored. */
static int exec_child(void) {
    struct ksigaction a;
    if (get_action(SIGSYS, &a) != 0) return 2;
    if (!is_default(&a)) return 3;
    if (get_action(SIGUSR1, &a) != 0) return 4;
    if (!is_default(&a)) return 5;
    if (get_action(SIGPIPE, &a) != 0) return 6;
    if (a.handler != SIG_IGN) return 7;
    return 0;
}

/* A fork(2) child: the handler is there; then, as a launcher does, put
 * SIGSYS back to SIG_DFL with the flags it had and exec. */
static __attribute__((noreturn)) void forked_child(int exec_after) {
    struct ksigaction a;
    if (get_action(SIGSYS, &a) != 0 || !same_as_installed(&a)) exit_group(2);
    if (exec_after) {
        struct ksigaction reset = a;
        reset.handler = SIG_DFL;
        if (set_action(SIGSYS, &reset, 0) != 0) exit_group(3);
        char *argv[] = {"sigsys", "exec-child", 0};
        char *envp[] = {0};
        syscall3(SYS_execve, (i64) "/proc/self/exe", (i64)argv, (i64)envp);
        exit_group(4);
    }
    /* The child's own change must not reach the parent. */
    struct ksigaction mine = {SIG_IGN, 0, 0, 0};
    if (set_action(SIGSYS, &mine, 0) != 0) exit_group(5);
    exit_group(0);
}

/* A clone(2) child sharing the memory and the parent's stack (vfork's
 * way): the parent is suspended until it ends. */
static __attribute__((noinline, noreturn)) void clone_child(void) {
    struct ksigaction a;
    if (get_action(SIGSYS, &a) != 0 || !same_as_installed(&a)) exit_group(2);
    exit_group(0);
}

int test_main(void) {
    u64 *sp = poly_initial_sp;
    char **argv = (char **)(sp + 1);
    if (sp[0] >= 2 && str_eq(argv[1], "exec-child")) return exec_child();

    struct ksigaction a, old;

    /* A new process: SIG_DFL, flags 0, empty mask. */
    i64 r = get_action(SIGSYS, &a);
    check("sigsys_starts_at_sig_dfl", r == 0 && is_default(&a), r);

    /* The old action of an install is the one before; querying gives the
     * installed one back, flags as set. */
    struct ksigaction mine = {(u64)handler, HANDLER_FLAGS, (u64)handler, 1UL << (SIGUSR1 - 1)};
    for (int i = 0; i < (int)sizeof old; i++) ((char *)&old)[i] = (char)0x55;
    r = set_action(SIGSYS, &mine, &old);
    check("sigsys_install_reports_the_default_as_old", r == 0 && is_default(&old), r);
    r = get_action(SIGSYS, &a);
    check("sigsys_reads_back_what_was_installed", r == 0 && same_as_installed(&a), r);
    r = set_action(SIGUSR1, &mine, 0);
    check("sigusr1_installed", r == 0, r);
    struct ksigaction ign = {SIG_IGN, 0, 0, 0};
    r = set_action(SIGPIPE, &ign, 0);
    check("sigpipe_ignored", r == 0, r);

    /* fork(2): the child has the handler, and its change stays its own. */
    i64 pid = syscall0(SYS_fork);
    if (pid == 0) forked_child(0);
    check("fork_child_sees_the_handler_and_its_change_stays_its_own", pid > 0 && wait_status(pid) == EXITED(0), pid);
    r = get_action(SIGSYS, &a);
    check("fork_leaves_the_parents_handler", r == 0 && same_as_installed(&a), r);

    /* fork(2) then execve(2) after putting SIGSYS back to SIG_DFL with the
     * handler's flags: the new program finds a new process's actions. */
    pid = syscall0(SYS_fork);
    if (pid == 0) forked_child(1);
    check("exec_resets_flags_and_handlers_and_keeps_ignored", pid > 0 && wait_status(pid) == EXITED(0), pid);

    /* clone(2) with CLONE_VM | CLONE_VFORK (a new process, no
     * CLONE_SIGHAND) and with CLONE_SIGHAND: either way the child starts
     * with the parent's actions. */
    pid = syscall6(SYS_clone, CLONE_VM | CLONE_VFORK | SIGCHLD, 0, 0, 0, 0, 0);
    if (pid == 0) clone_child();
    check("clone_without_sighand_child_sees_the_handler", pid > 0 && wait_status(pid) == EXITED(0), pid);
    pid = syscall6(SYS_clone, CLONE_VM | CLONE_VFORK | CLONE_SIGHAND | SIGCHLD, 0, 0, 0, 0, 0);
    if (pid == 0) clone_child();
    check("clone_with_sighand_child_sees_the_handler", pid > 0 && wait_status(pid) == EXITED(0), pid);

    r = get_action(SIGSYS, &a);
    check("parent_handler_after_all_children", r == 0 && same_as_installed(&a), r);
    return finish();
}
