/* A buffer that cannot take 8 bytes is EFAULT (14): NULL, address 1, a
 * kernel address, a non-canonical address, read-only memory (the
 * program's own code), a read-only page, a PROT_NONE page, and a buffer
 * starting 4 bytes before an unmapped page. (What lands in the writable
 * part of a buffer that faults half way is not specified.) The call still
 * works afterwards. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getaffinity 204

int test_main(void) {
    expect("null", syscall3(SYS_sched_getaffinity, 0, 8, 0), -EFAULT);
    expect("null_len128", syscall3(SYS_sched_getaffinity, 0, 128, 0), -EFAULT);
    expect("addr_1", syscall3(SYS_sched_getaffinity, 0, 8, 1), -EFAULT);
    expect("kernel_addr", syscall3(SYS_sched_getaffinity, 0, 8, 0xffff800000000000UL), -EFAULT);
    expect("non_canonical", syscall3(SYS_sched_getaffinity, 0, 8, 0x0000800000000000UL), -EFAULT);
    expect("own_code", syscall3(SYS_sched_getaffinity, 0, 8, (void *)test_main), -EFAULT);

    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap", (i64)pg > 0, (i64)pg);
    expect("unmap_second_page", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    expect("straddles_unmapped", syscall3(SYS_sched_getaffinity, 0, 8, pg + A_PAGE - 4), -EFAULT);
    expect("unmapped", syscall3(SYS_sched_getaffinity, 0, 8, pg + A_PAGE), -EFAULT);

    u64 *p = (u64 *)pg;
    *p = 0x99;
    expect("make_read_only", syscall3(SYS_mprotect, pg, A_PAGE, PROT_READ), 0);
    expect("read_only_page", syscall3(SYS_sched_getaffinity, 0, 8, p), -EFAULT);
    check("read_only_unchanged", *p == 0x99, (i64)*p);
    expect("make_prot_none", syscall3(SYS_mprotect, pg, A_PAGE, PROT_NONE), 0);
    expect("prot_none_page", syscall3(SYS_sched_getaffinity, 0, 8, p), -EFAULT);
    expect("make_writable", syscall3(SYS_mprotect, pg, A_PAGE, PROT_READ | PROT_WRITE), 0);
    check("prot_none_unchanged", *p == 0x99, (i64)*p);

    u64 m = 0;
    expect("works_after", syscall3(SYS_sched_getaffinity, 0, 8, &m), 8);
    check("works_after_mask", m != 0, 0);
    return finish();
}
