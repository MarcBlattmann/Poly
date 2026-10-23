/* The length: only min(len, 8) bytes are read. Shorter lengths read the
 * first len bytes and treat the rest as zero (len 1 with all ones names
 * CPUs 0-7); longer ones read 8 bytes, so a mask in the last 8 bytes of a
 * page followed by an unmapped page works with len 128 or 4096. len is a
 * 32-bit unsigned value: 0x100000000 is 0 (EINVAL), 0x100000008 is 8, and
 * -1 (0xffffffff) and 0x80000000 just read 8 bytes. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start;

    u64 v = ~0UL;
    expect("len1_all_ones", syscall3(SYS_sched_setaffinity, 0, 1, &v), 0);
    check("len1_low_byte_only", mask_of(0) == (start & 0xff), 0);
    unsigned char three[4] = {0xff, 0xff, 0xff, 0};
    expect("len3_all_ones", syscall3(SYS_sched_setaffinity, 0, 3, three), 0);
    check("len3_low_bytes_only", mask_of(0) == (start & 0xffffff), 0);

    char *pg = (char *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    expect("unmap_second_page", syscall2(SYS_munmap, pg + A_PAGE, A_PAGE), 0);
    u64 *tail = (u64 *)(pg + A_PAGE - 8);
    *tail = lo;
    expect("tail_len8", syscall3(SYS_sched_setaffinity, 0, 8, tail), 0);
    *tail = start;
    expect("tail_len128", syscall3(SYS_sched_setaffinity, 0, 128, tail), 0);
    check("tail_len128_seen", mask_of(0) == start, 0);
    *tail = lo;
    expect("tail_len4096", syscall3(SYS_sched_setaffinity, 0, 4096, tail), 0);
    check("tail_len4096_seen", mask_of(0) == lo, 0);
    *tail = start;
    expect("tail_len_minus_one", syscall3(SYS_sched_setaffinity, 0, -1L, tail), 0);
    check("tail_len_minus_one_seen", mask_of(0) == start, 0);
    *tail = lo;
    expect("tail_len_2g", syscall3(SYS_sched_setaffinity, 0, 0x80000000L, tail), 0);
    check("tail_len_2g_seen", mask_of(0) == lo, 0);

    v = start;
    expect("len_4g_is_zero", syscall3(SYS_sched_setaffinity, 0, 0x100000000L, &v), -EINVAL);
    check("len_4g_kept_mask", mask_of(0) == lo, 0);
    expect("len_4g_plus_8", syscall3(SYS_sched_setaffinity, 0, 0x100000008L, &v), 0);
    check("len_4g_plus_8_seen", mask_of(0) == start, 0);
    return finish();
}
