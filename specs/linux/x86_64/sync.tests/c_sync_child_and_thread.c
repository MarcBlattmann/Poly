/* sync() from other tasks: a fork child and a child sharing the memory
 * (CLONE_VM) both get 0, and so does the parent afterwards.
 * poly-diff: syscalls=off (child processes)
 */
#include "a_sys.h"

#define SYS_sync 162

static volatile i64 vm_result = 99;
static volatile i32 done;

static int vm_child(void *arg) {
    (void)arg;
    vm_result = syscall0(SYS_sync);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) exit_group(syscall0(SYS_sync) == 0 ? 0 : 3);
    i32 status = -1;
    expect("wait_fork", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("fork_child_ok", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);

    long t = a_spawn(0, &done, vm_child, 0);
    check("spawn", t > 0, t);
    if (t > 0) {
        expect("vm_child_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), 1);
        expect("vm_child_result", vm_result, 0);
        status = -1;
        expect("wait_vm", syscall4(SYS_wait4, t, &status, 0, 0), t);
    }
    expect("parent", syscall0(SYS_sync), 0);
    return finish();
}
