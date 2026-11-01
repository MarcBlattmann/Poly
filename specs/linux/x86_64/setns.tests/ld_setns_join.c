/* setns(fd, 0 or CLONE_NEWUTS / CLONE_NEWIPC) with a namespace file of
 * another namespace moves the calling process into it: after joining a
 * UTS namespace whose host name was changed, uname shows that name, and
 * joining back through a descriptor of the old namespace (kept open)
 * restores the old name. A namespace stays alive while a descriptor of
 * it is open, after its last process has exited. Joining an IPC
 * namespace makes its System V objects visible (a key made there is
 * found; the key outside is not). A descriptor of the namespace one is
 * already in may be joined again (0, nothing changes).
 * poly-diff: needs=root (new namespaces and setns need CAP_SYS_ADMIN)
 * poly-diff: syscalls=names (semaphore ids depend on the IPC namespace's history)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_semget 64
#define SYS_semctl 66
#define SYS_sethostname 170
#define SYS_unshare 272
#define SYS_setns 308
/* linux/sched.h */
#define CLONE_NEWUTS 0x04000000
#define CLONE_NEWIPC 0x08000000
/* linux/ipc.h */
#define IPC_CREAT 01000
#define IPC_RMID 0
#define KEY 0x4c440808

static struct a_utsname before, u;

static const char *nodename(void) {
    memset(&u, 0, sizeof u);
    syscall1(SYS_uname, &u);
    return u.nodename;
}

static int child(void) {
    i64 old_uts = a_open("/proc/self/ns/uts", O_RDONLY, 0);
    i64 old_ipc = a_open("/proc/self/ns/ipc", O_RDONLY, 0);
    expect("old_fds", old_uts * 10 + old_ipc, 34);

    /* A helper sharing the descriptor table makes the namespaces, names
     * the host, makes a semaphore set and leaves descriptors of them. */
    i64 c = syscall5(SYS_clone, CLONE_VFORK | CLONE_FILES | SIGCHLD, 0, 0, 0, 0);
    if (c == 0) {
        if (syscall1(SYS_unshare, CLONE_NEWUTS | CLONE_NEWIPC) != 0) syscall1(SYS_exit, 1);
        syscall2(SYS_sethostname, "joined", 6);
        syscall3(SYS_semget, KEY, 1, IPC_CREAT | 0600);
        i64 a = a_open("/proc/self/ns/uts", O_RDONLY, 0);
        i64 b = a_open("/proc/self/ns/ipc", O_RDONLY, 0);
        syscall1(SYS_exit, (int)(a * 10 + b));
    }
    i32 st = -1;
    syscall4(SYS_wait4, c, &st, 0, 0);
    expect("helper_fds", A_WEXITSTATUS(st), 56);

    expect("rejoin_own_uts", syscall2(SYS_setns, old_uts, 0), 0);
    check("still_old_name", str_eq(nodename(), before.nodename), 0);
    expect("join_uts_typed", syscall2(SYS_setns, 5, CLONE_NEWUTS), 0);
    check("name_joined", str_eq(nodename(), "joined"), 0);
    expect("join_back_type0", syscall2(SYS_setns, old_uts, 0), 0);
    check("name_old_again", str_eq(nodename(), before.nodename), 0);
    expect("join_uts_type0", syscall2(SYS_setns, 5, 0), 0);
    check("name_joined_again", str_eq(nodename(), "joined"), 0);

    expect("key_not_here", syscall3(SYS_semget, KEY, 0, 0), -ENOENT);
    expect("join_ipc", syscall2(SYS_setns, 6, CLONE_NEWIPC), 0);
    i64 id = syscall3(SYS_semget, KEY, 0, 0);
    check("key_found_after_join", id >= 0, id);
    expect("rmid_in_joined", syscall3(SYS_semctl, id, 0, IPC_RMID), 0);
    expect("join_back_ipc", syscall2(SYS_setns, old_ipc, CLONE_NEWIPC), 0);
    expect("key_not_here_again", syscall3(SYS_semget, KEY, 0, 0), -ENOENT);
    a_shared()[0] = passed;
    a_shared()[1] = failed;
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    syscall1(SYS_uname, &before);
    a_shared();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, child());
    i32 st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    expect("child_exit_0", st, 0);
    passed += a_shared()[0];
    failed += a_shared()[1];
    check("own_name_unchanged", str_eq(nodename(), before.nodename), 0);
    return finish();
}
