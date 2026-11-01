/* process_vm_readv partial transfers (on the caller itself, which is
 * allowed): the copy stops at the first remote or local byte that cannot
 * be accessed and returns the bytes copied so far; when nothing could be
 * copied (the first remote or local range starts in unmapped memory) the
 * result is EFAULT. An iovec of the remote list that is wholly unreadable
 * after a readable one ends the copy with the count so far. Read-only
 * remote memory can be read.
 * poly-diff: syscalls=names (the caller's pid is an argument)
 */
#include "a_sys.h"

#define SYS_process_vm_readv 310

static char buf[256];

static i64 rd(i64 pid, struct a_iovec *l, i64 nl, struct a_iovec *r, i64 nr) {
    return syscall6(SYS_process_vm_readv, pid, (i64)l, nl, (i64)r, nr, 0);
}

int test_main(void) {
    i64 me = syscall0(SYS_getpid);
    /* Two pages, the second unmapped. */
    char *m = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second_page", syscall2(SYS_munmap, m + A_PAGE, A_PAGE), 0);
    memset(m, 'x', A_PAGE);

    struct a_iovec l = {buf, 100};
    struct a_iovec r = {m + A_PAGE - 6, 100};
    expect("remote_runs_into_hole_6", rd(me, &l, 1, &r, 1), 6);
    struct a_iovec r2[2] = {{m + 4000, 50}, {m + A_PAGE, 10}};
    expect("second_remote_unmapped_50", rd(me, &l, 1, r2, 2), 50);
    struct a_iovec r3[2] = {{m + A_PAGE, 10}, {m, 50}};
    expect("first_remote_unmapped_efault", rd(me, &l, 1, r3, 2), -EFAULT);
    struct a_iovec rnull = {0, 10};
    expect("remote_null_efault", rd(me, &l, 1, &rnull, 1), -EFAULT);

    static char src[100] = "source";
    struct a_iovec rs = {src, 100};
    struct a_iovec lh = {m + A_PAGE - 6, 100};
    expect("local_runs_into_hole_6", rd(me, &lh, 1, &rs, 1), 6);
    check("local_partial_data", mem_eq(m + A_PAGE - 6, "source", 6), m[A_PAGE - 6]);
    struct a_iovec lbad = {m + A_PAGE, 10};
    expect("local_unmapped_efault", rd(me, &lbad, 1, &rs, 1), -EFAULT);
    struct a_iovec l2[2] = {{buf, 4}, {m + A_PAGE, 10}};
    expect("second_local_unmapped_4", rd(me, l2, 2, &rs, 1), 4);

    /* Read-only memory is readable; the local side must be writable. */
    char *ro = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    struct a_iovec rro = {ro, 16};
    memset(buf, 1, 16);
    expect("read_readonly_16", rd(me, &l, 1, &rro, 1), 16);
    check("readonly_zeros", buf[0] == 0 && buf[15] == 0, buf[0]);
    struct a_iovec lro = {ro, 16};
    expect("local_readonly_efault", rd(me, &lro, 1, &rs, 1), -EFAULT);
    char *none = (char *)syscall6(SYS_mmap, 0, A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    struct a_iovec rnone = {none, 16};
    expect("remote_prot_none_efault", rd(me, &l, 1, &rnone, 1), -EFAULT);
    return finish();
}
