/* kcmp needs ptrace read access to both processes: a child that switched
 * to uid/gid 4242 may compare itself with itself, but kcmp involving its
 * root parent is EPERM, for every type, checked before the type (an
 * invalid type is still EPERM) and after the pid lookup (a pid with no
 * process is still ESRCH).
 * poly-diff: needs=root (switches to another uid)
 * poly-diff: syscalls=names (pids are arguments)
 */
#include "a_sys.h"

#define SYS_setgroups 116
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_kcmp 312
#define KCMP_FILE 0
#define KCMP_VM 1
#define KCMP_FILES 2
#define NO_PID 0x7fffffff

static i64 kcmp(i64 p1, i64 p2, i64 type, i64 i1, i64 i2) { return syscall5(SYS_kcmp, p1, p2, type, i1, i2); }

int test_main(void) {
    a_block_sigchld();
    i64 parent = syscall0(SYS_getpid);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        syscall2(SYS_setgroups, 0, 0);
        syscall3(SYS_setresgid, 4242, 4242, 4242);
        if (syscall3(SYS_setresuid, 4242, 4242, 4242) != 0) syscall1(SYS_exit, 9);
        i64 me = syscall0(SYS_getpid);
        expect("self_vm", kcmp(me, me, KCMP_VM, 0, 0), 0);
        expect("self_file", kcmp(me, me, KCMP_FILE, 1, 1), 0);
        expect("parent_vm_eperm", kcmp(me, parent, KCMP_VM, 0, 0), -EPERM);
        expect("parent_first_eperm", kcmp(parent, me, KCMP_FILES, 0, 0), -EPERM);
        expect("parent_file_eperm", kcmp(me, parent, KCMP_FILE, 1, 1), -EPERM);
        expect("eperm_before_type", kcmp(me, parent, 100, 0, 0), -EPERM);
        expect("esrch_before_eperm", kcmp(NO_PID, parent, KCMP_VM, 0, 0), -ESRCH);
        syscall1(SYS_exit, finish());
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    expect("child_passed", status, 0);
    return finish();
}
