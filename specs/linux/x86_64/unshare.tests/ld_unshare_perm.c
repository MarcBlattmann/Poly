/* Without CAP_SYS_ADMIN, unshare of the mount, UTS, IPC, network, PID,
 * cgroup and time namespaces is EPERM, alone or together with flags that
 * would work (nothing is unshared then: the descriptor table stays
 * shared, see below), while an invalid bit still makes it EINVAL. The
 * non-namespace flags work. Run as a normal user (root first becomes
 * uid/gid 65534 without groups in a child), so the output is the same for
 * every caller. CLONE_NEWUSER is left out: whether a normal user may
 * create user namespaces is system configuration.
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_unshare 272
/* linux/sched.h */
#define CLONE_NEWTIME 0x80
#define CLONE_NEWNS 0x20000
#define CLONE_NEWCGROUP 0x02000000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWPID 0x20000000
#define CLONE_NEWNET 0x40000000

static int child(void) {
    if (syscall0(SYS_geteuid) == 0) {
        if (syscall2(SYS_setgroups, 0, 0) || syscall3(SYS_setresgid, 65534, 65534, 65534) ||
            syscall3(SYS_setresuid, 65534, 65534, 65534))
            return 2;
    }
    expect("newns_eperm", syscall1(SYS_unshare, CLONE_NEWNS), -EPERM);
    expect("newuts_eperm", syscall1(SYS_unshare, CLONE_NEWUTS), -EPERM);
    expect("newipc_eperm", syscall1(SYS_unshare, CLONE_NEWIPC), -EPERM);
    expect("newnet_eperm", syscall1(SYS_unshare, CLONE_NEWNET), -EPERM);
    expect("newpid_eperm", syscall1(SYS_unshare, CLONE_NEWPID), -EPERM);
    expect("newcgroup_eperm", syscall1(SYS_unshare, CLONE_NEWCGROUP), -EPERM);
    expect("newtime_eperm", syscall1(SYS_unshare, CLONE_NEWTIME), -EPERM);
    expect("newuts_bad_bit_einval", syscall1(SYS_unshare, CLONE_NEWUTS | 0x1000), -EINVAL);
    expect("plain_flags_ok", syscall1(SYS_unshare, CLONE_FS | CLONE_FILES | CLONE_SYSVSEM), 0);

    /* With a shared descriptor table, a refused unshare leaves it shared. */
    i64 c = syscall5(SYS_clone, CLONE_VFORK | CLONE_FILES | SIGCHLD, 0, 0, 0, 0);
    if (c == 0) {
        i64 r = syscall1(SYS_unshare, CLONE_FILES | CLONE_NEWUTS);
        i64 fd = a_open("/", O_RDONLY | O_DIRECTORY, 0);
        syscall1(SYS_exit, r == -EPERM ? (int)fd : 99);
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    i64 fd = A_WEXITSTATUS(st);
    expect("refused_child_fd", fd, 3);
    expect("table_still_shared", syscall2(SYS_fcntl, 3, F_GETFD), 0);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_shared();
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
