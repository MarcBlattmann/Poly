/* setns without CAP_SYS_ADMIN is EPERM, even to re-enter the namespace
 * the caller is already in, for the uts, ipc, net, mnt, pid, cgroup and
 * time namespace files, with nstype 0 or the matching type; but a wrong
 * nstype is EINVAL (checked first) and one's own user namespace is
 * EINVAL. With a pidfd of itself and a set of CLONE_NEW* flags the call
 * is EPERM as well. Run as a normal user (root first becomes uid/gid
 * 65534 without groups in a child), so the output is the same for every
 * caller.
 * poly-diff: syscalls=names (pidfd_open takes the test's pid)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_setns 308
#define SYS_pidfd_open 434
/* linux/sched.h */
#define CLONE_NEWTIME 0x80
#define CLONE_NEWNS 0x20000
#define CLONE_NEWCGROUP 0x02000000
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
#define CLONE_NEWUSER 0x10000000
#define CLONE_NEWPID 0x20000000
#define CLONE_NEWNET 0x40000000

static const char *names[] = {"uts", "ipc", "net", "mnt", "pid", "cgroup", "time", 0};
static const i64 types[] = {CLONE_NEWUTS, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWCGROUP,
                            CLONE_NEWTIME};

static int child(void) {
    if (syscall0(SYS_geteuid) == 0) {
        if (syscall2(SYS_setgroups, 0, 0) || syscall3(SYS_setresgid, 65534, 65534, 65534) ||
            syscall3(SYS_setresuid, 65534, 65534, 65534))
            return 2;
    }
    for (int i = 0; names[i]; i++) {
        char path[48], label[48];
        str_cat2(path, "/proc/self/ns/", names[i]);
        i64 fd = a_open(path, O_RDONLY, 0);
        str_cat2(label, names[i], "_open");
        check(label, fd >= 0, fd);
        str_cat2(label, names[i], "_type0_eperm");
        expect(label, syscall2(SYS_setns, fd, 0), -EPERM);
        str_cat2(label, names[i], "_typed_eperm");
        expect(label, syscall2(SYS_setns, fd, types[i]), -EPERM);
        str_cat2(label, names[i], "_wrong_type_einval");
        expect(label, syscall2(SYS_setns, fd, i ? CLONE_NEWUTS : CLONE_NEWIPC), -EINVAL);
        a_close(fd);
    }
    i64 user = a_open("/proc/self/ns/user", O_RDONLY, 0);
    expect("own_userns_einval", syscall2(SYS_setns, user, 0), -EINVAL);
    a_close(user);

    i64 pidfd = syscall2(SYS_pidfd_open, syscall0(SYS_getpid), 0);
    check("pidfd_open", pidfd >= 0, pidfd);
    expect("pidfd_uts_eperm", syscall2(SYS_setns, pidfd, CLONE_NEWUTS), -EPERM);
    expect("pidfd_uts_ipc_net_eperm", syscall2(SYS_setns, pidfd, CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET), -EPERM);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    return finish();
}
