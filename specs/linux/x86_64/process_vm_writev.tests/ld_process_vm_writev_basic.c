/* process_vm_writev(pid, local_iov, liovcnt, remote_iov, riovcnt, 0)
 * copies from the caller's memory into another process's: here a child
 * made by fork, which then sees the new bytes in its own copy while the
 * caller's copy is unchanged. Local ranges are read in order and written
 * to the remote ranges in order (scatter/gather); the result is the number
 * of bytes copied, the smaller of the two totals.
 * poly-diff: syscalls=off (two processes run at once; the child's pid is an argument)
 */
#include "a_sys.h"

#define SYS_process_vm_writev 311

static char data[32] = "................................";

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        sh[0] = 1;
        while (sh[0] != 2) a_relax();
        /* Report what the parent wrote: 1 if as expected. */
        sh[1] = mem_eq(data, "HELLO...WORLD!..", 16) && mem_eq(data + 16, "ab..............", 16);
        sh[0] = 3;
        while (sh[0] != 4) a_relax();
        syscall1(SYS_exit, 0);
    }
    if (!a_spin_until(&sh[0], 1, A_SPIN_LIMIT)) return finish();

    static char w1[8] = "HELLO", w2[8] = "WORLD!";
    struct a_iovec l = {w1, 5}, r = {data, 5};
    expect("write_5", syscall6(SYS_process_vm_writev, pid, (i64)&l, 1, (i64)&r, 1, 0), 5);
    /* Two local ranges into two remote ranges with other boundaries. */
    struct a_iovec ls[2] = {{w2, 6}, {w1 + 3, 2}};
    struct a_iovec rs[2] = {{data + 8, 4}, {data + 12, 2}};
    expect("gather_6", syscall6(SYS_process_vm_writev, pid, (i64)ls, 2, (i64)rs, 2, 0), 6);
    /* The smaller total wins: 2 of 6 local bytes go out. */
    static char w3[8] = "abcdef";
    struct a_iovec l3 = {w3, 6}, r3 = {data + 16, 2};
    expect("remote_shorter_2", syscall6(SYS_process_vm_writev, pid, (i64)&l3, 1, (i64)&r3, 1, 0), 2);
    check("own_copy_unchanged", data[0] == '.' && data[8] == '.', data[0]);

    sh[0] = 2;
    a_spin_until(&sh[0], 3, A_SPIN_LIMIT);
    expect("child_saw_writes", sh[1], 1);
    sh[0] = 4;
    i32 status = -1;
    expect("child_reaped", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_status", status, 0);
    return finish();
}
