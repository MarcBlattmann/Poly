/* The first process of a PID namespace made by clone(CLONE_NEWPID).
 * Inside it sees itself as pid 1 and tid 1 with parent 0; its children
 * are numbered 2, 3, 4 ...; wait4 on them by number returns their exit
 * statuses; after waiting for all, wait4(-1) is ECHILD and kill(2, 0) is
 * ESRCH while kill(1, 0) (itself) is 0. Its /proc/self/ns/pid is another
 * namespace than the parent's and equals its pid_for_children. Seen
 * through the initial /proc (which the test uses unchanged) the same
 * process has a different number: /proc/self/status shows "NSpid:" with
 * two numbers (the outer one, and 1), "NSpgid:" and "NSsid:" with two,
 * "Pid:" equal to the first NSpid field and not 1, and "PPid:" equal to
 * the creator's own pid. pidfd_open works for pids of the namespace (2)
 * and fails with ESRCH for a number that does not exist in it (the
 * creator's number is not visible there).
 * poly-diff: needs=root (a new PID namespace needs CAP_SYS_ADMIN)
 * poly-diff: syscalls=off (several processes, pids in the output paths)
 */
#include "p_ns.h"

static i64 creator_pid;
static i64 outer_pid_ns;
static char buf[2048];

static int child_exit(int code) {
    syscall1(SYS_exit_group, code);
    return 0;
}

static int init_body(void) {
    expect("getpid_1", syscall0(SYS_getpid), 1);
    expect("gettid_1", syscall0(SYS_gettid), 1);
    expect("getppid_0", syscall0(SYS_getppid), 0);

    i64 p1 = p_fork();
    if (p1 == 0) child_exit(11);
    i64 p2 = p_fork();
    if (p2 == 0) child_exit(12);
    expect("first_child_pid_2", p1, 2);
    expect("second_child_pid_3", p2, 3);
    i32 st = -1;
    expect("wait_pid_3", syscall4(SYS_wait4, 3, &st, 0, 0), 3);
    expect("status_pid_3", st, P_EXITED(12));
    expect("wait_pid_2", syscall4(SYS_wait4, 2, &st, 0, 0), 2);
    expect("status_pid_2", st, P_EXITED(11));
    expect("wait_no_children", syscall4(SYS_wait4, -1, &st, 0, 0), -ECHILD);
    expect("kill_gone_child", syscall2(SYS_kill, 2, 0), -ESRCH);
    expect("kill_self_0", syscall2(SYS_kill, 1, 0), 0);

    i32 hold[2];
    p_pipe(hold);
    i64 p3 = p_fork();
    if (p3 == 0) {
        char c;
        a_read(hold[0], &c, 1);
        child_exit(0);
    }
    expect("third_child_pid_4", p3, 4);
    i64 pfd = syscall2(SYS_pidfd_open, 4, 0);
    check("pidfd_open_inner_pid", pfd >= 0, pfd);
    expect("pidfd_open_missing", syscall2(SYS_pidfd_open, 999999, 0), -ESRCH);
    expect("pidfd_open_creator_not_visible", syscall2(SYS_pidfd_open, 1000000 + creator_pid, 0), -ESRCH);
    a_write(hold[1], "x", 1);
    expect("wait_pid_4", syscall4(SYS_wait4, 4, &st, 0, 0), 4);
    expect("status_pid_4", st, 0);

    check("pid_ns_differs", p_ns_ino("/proc/self/ns/pid") != outer_pid_ns, 0);
    expect("pid_for_children_is_own", p_ns_ino("/proc/self/ns/pid_for_children"), p_ns_ino("/proc/self/ns/pid"));

    p_read_file("/proc/self/status", buf, sizeof buf);
    const char *nspid = p_status_field(buf, "NSpid");
    check("nspid_present", nspid != P_NULL, 0);
    expect("nspid_two_fields", nspid ? p_fields(nspid) : -1, 2);
    expect("nspid_last_1", nspid ? (i64)p_last_field(nspid) : -1, 1);
    const char *pid = p_status_field(buf, "Pid");
    check("pid_not_1", pid && p_num(pid) != 1, 0);
    check("pid_is_first_nspid", pid && nspid && p_num(pid) == p_num(nspid), 0);
    const char *nspgid = p_status_field(buf, "NSpgid");
    const char *nssid = p_status_field(buf, "NSsid");
    expect("nspgid_two_fields", nspgid ? p_fields(nspgid) : -1, 2);
    expect("nssid_two_fields", nssid ? p_fields(nssid) : -1, 2);
    const char *ppid = p_status_field(buf, "PPid");
    check("ppid_is_creator", ppid && p_num(ppid) == (u64)creator_pid, ppid ? (i64)p_num(ppid) : -1);
    return failed ? 1 : 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    creator_pid = syscall0(SYS_getpid);
    outer_pid_ns = p_ns_ino("/proc/self/ns/pid");
    i64 child = p_clone(CLONE_NEWPID);
    if (child == 0) {
        passed = failed = 0;
        int rc = init_body();
        p_report();
        p_exit(rc);
    }
    int st = p_wait(child);
    check("outer_pid_not_1", child > 1, child);
    expect("init_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
