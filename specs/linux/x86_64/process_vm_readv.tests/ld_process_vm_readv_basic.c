/* process_vm_readv(pid, local_iov, liovcnt, remote_iov, riovcnt, 0)
 * copies from another process's memory into the caller's: here a child
 * made by fork (its own copy of memory) that changed its copy of a buffer
 * after the fork. The remote ranges are read in order and written to the
 * local ranges in order (scatter/gather; the iovec boundaries on the two
 * sides need not match); the result is the number of bytes copied, the
 * smaller of the two totals. Zero-length iovecs are skipped. The caller's
 * own copy of the buffer is unchanged.
 * poly-diff: syscalls=off (two processes run at once; the child's pid is an argument)
 */
#include "a_sys.h"

#define SYS_process_vm_readv 310

static char data[32] = "parent-data-0123456789";
static char out[64];

int test_main(void) {
    a_block_sigchld();
    volatile i32 *sh = a_shared();
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) {
        memcpy(data, "CHILD-DATA-abcdefghij", 22);
        sh[0] = 1;
        while (sh[0] != 2) a_relax();
        syscall1(SYS_exit, 0);
    }
    if (!a_spin_until(&sh[0], 1, A_SPIN_LIMIT)) return finish();

    struct a_iovec l = {out, 22}, r = {data, 22};
    expect("read_22", syscall6(SYS_process_vm_readv, pid, (i64)&l, 1, (i64)&r, 1, 0), 22);
    check("got_child_copy", mem_eq(out, "CHILD-DATA-abcdefghij", 22), out[0]);
    check("own_copy_unchanged", mem_eq(data, "parent-data-0123456789", 22), data[0]);

    /* Gather two remote ranges into three local ones. */
    memset(out, '.', sizeof out);
    struct a_iovec ls[3] = {{out, 3}, {out + 10, 0}, {out + 20, 10}};
    struct a_iovec rs[2] = {{data, 6}, {data + 11, 7}};
    expect("scatter_13", syscall6(SYS_process_vm_readv, pid, (i64)ls, 3, (i64)rs, 2, 0), 13);
    check("scatter_first", mem_eq(out, "CHI...", 6), out[3]);
    check("scatter_second", mem_eq(out + 20, "LD-abcdefg.", 11), out[20]);

    /* The smaller total wins. */
    struct a_iovec lshort = {out, 4};
    expect("local_shorter_4", syscall6(SYS_process_vm_readv, pid, (i64)&lshort, 1, (i64)&r, 1, 0), 4);
    struct a_iovec rshort = {data + 6, 5};
    expect("remote_shorter_5", syscall6(SYS_process_vm_readv, pid, (i64)&l, 1, (i64)&rshort, 1, 0), 5);
    check("remote_shorter_data", mem_eq(out, "DATA-", 5), out[0]);
    struct a_iovec z = {data, 0};
    expect("zero_length_0", syscall6(SYS_process_vm_readv, pid, (i64)&l, 1, (i64)&z, 1, 0), 0);
    expect("no_iovecs_0", syscall6(SYS_process_vm_readv, pid, (i64)&l, 0, (i64)&r, 1, 0), 0);

    sh[0] = 2;
    i32 status = -1;
    expect("child_reaped", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect("child_status", status, 0);
    return finish();
}
