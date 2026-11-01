/* A shared futex (no FUTEX2_PRIVATE) on MAP_SHARED memory works between
 * processes: a child made by fork waits on a word of a shared page; the
 * parent's private wake does not find it (another address space, returns
 * 0), its shared wake wakes it (returns 1) and the child's futex_wait
 * returns 0. A wake through a second mapping of the same page (another
 * address, same memory) finds the waiter too.
 * poly-diff: syscalls=off (two processes run at once)
 */
#include "a_sys.h"

#define SC6(n, a, b, c, d, e, f) syscall6((n), (i64)(a), (i64)(b), (i64)(c), (i64)(d), (i64)(e), (i64)(f))

#define SYS_futex 202
#define SYS_futex_wake 454
#define SYS_futex_wait 455
#define SYS_memfd_create 319
/* linux/futex.h */
#define FUTEX_CMP_REQUEUE 4
#define FUTEX2_SIZE_U32 0x02
#define FUTEX2_PRIVATE 128

static i64 queued(u32 *p, u32 val) {
    return SC6(SYS_futex, p, FUTEX_CMP_REQUEUE, 0, 0x7fffffff, p, val);
}

static int wait_queued(u32 *p, u32 val, i64 n) {
    u64 start = a_rdtsc();
    for (;;) {
        i64 q = queued(p, val);
        if (q == n) return 1;
        if (q > n || a_rdtsc() - start > A_SPIN_LIMIT) return 0;
        syscall0(SYS_sched_yield);
    }
}

static int child_wait(u32 *w) {
    i64 r = SC6(SYS_futex_wait, w, 3, 0xffffffff, FUTEX2_SIZE_U32, 0, 0);
    return r == 0 ? 0 : 10;
}

static void round_trip(const char *tag, u32 *wait_at, u32 *wake_at) {
    i64 pid = syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0);
    if (pid == 0) syscall1(SYS_exit_group, child_wait(wait_at));
    char name[64];
    check(str_cat2(name, tag, "_queued"), wait_queued(wait_at, 3, 1), 0);
    expect(str_cat2(name, tag, "_private_wake_0"),
           syscall4(SYS_futex_wake, wake_at, 0xffffffff, 1, FUTEX2_SIZE_U32 | FUTEX2_PRIVATE), 0);
    expect(str_cat2(name, tag, "_shared_wake_1"), syscall4(SYS_futex_wake, wake_at, 0xffffffff, 1, FUTEX2_SIZE_U32), 1);
    i32 status = -1;
    expect(str_cat2(name, tag, "_reaped"), syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    expect(str_cat2(name, tag, "_child_wait_returned_0"), status, 0);
}

int test_main(void) {
    a_block_sigchld();
    u32 *page = (u32 *)SC6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    page[10] = 3;
    round_trip("anon", page + 10, page + 10);

    /* Two mappings of one memfd page: the futex is the memory, not the address. */
    i64 fd = syscall2(SYS_memfd_create, "ld_futex_wake", 0);
    syscall2(SYS_ftruncate, fd, A_PAGE);
    u32 *m1 = (u32 *)SC6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    u32 *m2 = (u32 *)SC6(SYS_mmap, 0, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("two_addresses", m1 != m2, 0);
    m1[7] = 3;
    expect("same_memory", m2[7], 3);
    round_trip("alias", m1 + 7, m2 + 7);
    a_close(fd);
    return finish();
}
