/* MEMBARRIER_CMD_GLOBAL (1) and GLOBAL_EXPEDITED (2) need no
 * registration by the caller and return 0; flags non-zero is EINVAL; the
 * cpu_id argument is ignored. Registrations belong to the process (its
 * memory): a CLONE_VM task sees the parent's registration and may use
 * PRIVATE_EXPEDITED; a fork child inherits the registrations of its
 * parent (GET_REGISTRATIONS the same, PRIVATE_EXPEDITED works); what it
 * registers itself does not show in the parent.
 */
#include "a_sys.h"

#define SYS_membarrier 324
/* linux/membarrier.h */
#define MEMBARRIER_CMD_GLOBAL 1
#define MEMBARRIER_CMD_GLOBAL_EXPEDITED 2
#define MEMBARRIER_CMD_REGISTER_GLOBAL_EXPEDITED 4
#define MEMBARRIER_CMD_PRIVATE_EXPEDITED 8
#define MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED 16
#define MEMBARRIER_CMD_GET_REGISTRATIONS 512

static i64 mb(u64 cmd, u64 flags, u64 cpu) { return syscall3(SYS_membarrier, cmd, flags, cpu); }

static volatile i32 done;
static volatile i64 task_reg = -1, task_priv = -1;

static int task(void *arg) {
    (void)arg;
    task_reg = mb(MEMBARRIER_CMD_GET_REGISTRATIONS, 0, 0);
    task_priv = mb(MEMBARRIER_CMD_PRIVATE_EXPEDITED, 0, 0);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("global", mb(MEMBARRIER_CMD_GLOBAL, 0, 0), 0);
    expect("global_cpu_ignored", mb(MEMBARRIER_CMD_GLOBAL, 0, 99), 0);
    expect("global_flags_einval", mb(MEMBARRIER_CMD_GLOBAL, 1, 0), -EINVAL);
    expect("global_expedited_unregistered", mb(MEMBARRIER_CMD_GLOBAL_EXPEDITED, 0, 0), 0);
    expect("global_expedited_flags_einval", mb(MEMBARRIER_CMD_GLOBAL_EXPEDITED, 1, 0), -EINVAL);
    expect("register_global_expedited", mb(MEMBARRIER_CMD_REGISTER_GLOBAL_EXPEDITED, 0, 0), 0);
    expect("global_expedited_registered", mb(MEMBARRIER_CMD_GLOBAL_EXPEDITED, 0, 0), 0);
    expect("register_private", mb(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED, 0, 0), 0);
    expect("registrations", mb(MEMBARRIER_CMD_GET_REGISTRATIONS, 0, 0), 4 | 16);

    long pid = a_spawn(0, &done, task, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    expect("clone_vm_task_sees_registrations", task_reg, 4 | 16);
    expect("clone_vm_task_private_ok", task_priv, 0);

    i64 child = a_fork_sync();
    if (child == 0) {
        int code = 0;
        if (mb(MEMBARRIER_CMD_GET_REGISTRATIONS, 0, 0) != (4 | 16)) code |= 1;
        if (mb(MEMBARRIER_CMD_PRIVATE_EXPEDITED, 0, 0) != 0) code |= 2;
        if (mb(64, 0, 0) != 0) code |= 8; /* REGISTER_PRIVATE_EXPEDITED_SYNC_CORE */
        if (mb(MEMBARRIER_CMD_GLOBAL_EXPEDITED, 0, 0) != 0) code |= 4;
        syscall1(SYS_exit, code);
    }
    i32 status = -1;
    syscall4(SYS_wait4, child, &status, 0, 0);
    expect("fork_child_inherits_registrations", A_WEXITSTATUS(status) & 1, 0);
    expect("fork_child_private_ok", A_WEXITSTATUS(status) & 2, 0);
    expect("fork_child_global_expedited_ok", A_WEXITSTATUS(status) & 4, 0);
    expect("fork_child_registers_own", A_WEXITSTATUS(status) & 8, 0);
    expect("parent_registrations_kept", mb(MEMBARRIER_CMD_GET_REGISTRATIONS, 0, 0), 4 | 16);
    return finish();
}
