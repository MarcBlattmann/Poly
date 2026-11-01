/* process_vm_readv needs ptrace attach access to the target: a child
 * that switched to uid/gid 4242 may read its own memory but reading its
 * root parent's is EPERM (a pid with no process is still ESRCH).
 * poly-diff: needs=root (switches to another uid)
 * poly-diff: syscalls=names (pids are arguments)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_process_vm_readv 310
#define NO_PID 0x7fffffff

static char src[16] = "root secret", dst[16];

static i64 rd(i64 pid, struct a_iovec *l, struct a_iovec *r) {
    return syscall6(SYS_process_vm_readv, pid, (i64)l, 1, (i64)r, 1, 0);
}

int test_main(void) {
    a_block_sigchld();
    i64 parent = syscall0(SYS_getpid);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4242, 4242, 4242);
        if (syscall3(SYS_setresuid, 4242, 4242, 4242) != 0) syscall1(SYS_exit, 9);
        struct a_iovec l = {dst, 11}, r = {src, 11};
        expect("user_self_ok", rd(syscall0(SYS_getpid), &l, &r), 11);
        expect("user_parent_eperm", rd(parent, &l, &r), -EPERM);
        expect("user_no_pid_esrch", rd(NO_PID, &l, &r), -ESRCH);
        syscall1(SYS_exit, finish());
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_passed", status, 0);
    return finish();
}
