/* A registration is per thread. A fork child (its own copy of memory)
 * inherits it: registering the same area again in the child is EBUSY
 * and the child can unregister it, which does not affect the parent. A
 * task made with CLONE_VM (sharing memory) starts with no registration:
 * it can register an area of its own (0) and unregister it, and the
 * parent's registration is untouched.
 */
#include "a_sys.h"

#define SYS_rseq 334
#define RSEQ_FLAG_UNREGISTER 1
#define SIG 0x53053053

struct k_rseq {
    u32 cpu_id_start;
    u32 cpu_id;
    u64 rseq_cs;
    u32 flags;
    u32 node_id;
    u32 mm_cid;
    u32 pad;
} __attribute__((aligned(32)));

static struct k_rseq rs, rs2;
static volatile i32 done;
static volatile i64 results[4];

static i64 rseq(void *a, u64 len, u64 flags, u64 sig) { return syscall4(SYS_rseq, a, len, flags, sig); }

static int shared_child(void *arg) {
    (void)arg;
    results[0] = rseq(&rs2, 32, RSEQ_FLAG_UNREGISTER, SIG);
    results[1] = rseq(&rs2, 32, 0, SIG);
    results[2] = rseq(&rs2, 32, RSEQ_FLAG_UNREGISTER, SIG);
    results[3] = rseq(&rs, 32, RSEQ_FLAG_UNREGISTER, SIG);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    expect("register", rseq(&rs, 32, 0, SIG), 0);

    i64 pid = a_fork_sync();
    if (pid == 0) {
        int code = 0;
        if (rseq(&rs, 32, 0, SIG) != -EBUSY) code |= 1;
        if (rseq(&rs, 32, RSEQ_FLAG_UNREGISTER, SIG) != 0) code |= 2;
        if (rseq(&rs, 32, RSEQ_FLAG_UNREGISTER, SIG) != -EINVAL) code |= 4;
        syscall1(SYS_exit, code);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    check("fork_child_inherits_registration", A_WIFEXITED(status) && (A_WEXITSTATUS(status) & 1) == 0, status);
    check("fork_child_unregisters", A_WIFEXITED(status) && (A_WEXITSTATUS(status) & 6) == 0, status);
    expect("parent_still_registered", rseq(&rs, 32, 0, SIG), -EBUSY);

    pid = a_spawn(0, &done, shared_child, 0);
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    syscall4(SYS_wait4, pid, 0, 0, 0);
    expect("clone_vm_child_has_none", results[0], -EINVAL);
    expect("clone_vm_child_registers_own", results[1], 0);
    expect("clone_vm_child_unregisters_own", results[2], 0);
    expect("clone_vm_child_cannot_touch_parent", results[3], -EINVAL);
    expect("parent_registration_kept", rseq(&rs, 32, 0, SIG), -EBUSY);
    expect("parent_unregister", rseq(&rs, 32, RSEQ_FLAG_UNREGISTER, SIG), 0);
    return finish();
}
