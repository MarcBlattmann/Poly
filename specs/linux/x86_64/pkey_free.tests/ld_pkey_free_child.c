/* Without protection keys, pkey_free is EINVAL (22) for every key in a
 * child made by fork as well as in a thread-like child sharing memory
 * (CLONE_VM): no keys are inherited or shared, because none can be
 * allocated (pkey_alloc ENOSPC 28 in both). On a CPU with OSPKE the test
 * prints a skip line and passes.
 */
#include "a_sys.h"

#define SYS_pkey_alloc 330
#define SYS_pkey_free 331
#define ENOSPC 28

static int has_ospke(void) {
    u32 a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(0), "c"(0));
    if (a < 7) return 0;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(7), "c"(0));
    return (c >> 4) & 1;
}

static volatile i32 result = -1;

static int keys_absent(void) {
    int bad = 0;
    if (syscall2(SYS_pkey_alloc, 0, 0) != -ENOSPC) bad |= 1;
    for (int k = 0; k < 16; k++)
        if (syscall1(SYS_pkey_free, k) != -EINVAL) bad |= 2;
    return bad;
}

static int vm_child(void *arg) {
    (void)arg;
    result = keys_absent();
    return 0;
}

int test_main(void) {
    if (has_ospke()) {
        put("skip: this CPU has protection keys (OSPKE); the test covers CPUs without them\n");
        return 0;
    }
    expect("parent", keys_absent(), 0);
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit, keys_absent());
    i32 status = -1;
    expect("wait_fork_child", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("fork_child_exited", A_WIFEXITED(status), status);
    expect("fork_child", A_WEXITSTATUS(status), 0);

    static volatile i32 done;
    i64 c = a_spawn(0, &done, vm_child, 0);
    /* Wait for the child's end without system calls before reporting
     * anything, so its calls always come before the parent's next one. */
    int ended = c > 0 && a_spin_until(&done, 0, A_SPIN_LIMIT);
    check("spawn", c > 0, c);
    check("vm_child_done", ended, done);
    expect("vm_child", result, 0);
    expect("reap", syscall4(SYS_wait4, c, &status, 0, 0), c);
    return finish();
}
