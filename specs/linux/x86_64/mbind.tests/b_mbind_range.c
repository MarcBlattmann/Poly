/* The address range: addr must be page aligned (EINVAL, even with len 0);
 * len is rounded up to whole pages; a range that wraps past the top of
 * the address space is EINVAL; a len that rounds up to 0 (len 0, or
 * len >= 2^64-4095) succeeds at once without looking at the range or the
 * nodes. A range that touches unmapped memory is EFAULT.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_mbind 237
#define SYS_get_mempolicy 239
/* linux/mempolicy.h */
#define MPOL_DEFAULT 0
#define MPOL_PREFERRED 1
#define MPOL_BIND 2
#define MPOL_INTERLEAVE 3
#define MPOL_LOCAL 4
#define MPOL_PREFERRED_MANY 5
#define MPOL_WEIGHTED_INTERLEAVE 6
#define MPOL_F_STATIC_NODES (1 << 15)
#define MPOL_F_RELATIVE_NODES (1 << 14)
#define MPOL_F_NUMA_BALANCING (1 << 13)
#define MPOL_MF_STRICT (1 << 0)
#define MPOL_MF_MOVE (1 << 1)
#define MPOL_MF_MOVE_ALL (1 << 2)
#define MPOL_F_ADDR (1 << 1)

#define PAGE 4096

static A_UNUSED i64 mbind(void *addr, u64 len, i64 mode, const void *nodes, u64 maxnode, i64 flags) {
    return syscall6(SYS_mbind, (i64)addr, (i64)len, mode, (i64)nodes, (i64)maxnode, flags);
}

/* `pages` read-write private anonymous pages with a PROT_NONE page on
 * each side, so the neighbours of the range are known. */
static A_UNUSED char *region(int pages) {
    char *g = (char *)syscall6(SYS_mmap, 0, (pages + 2) * PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    return (char *)syscall6(SYS_mmap, (i64)(g + PAGE), pages * PAGE, PROT_READ | PROT_WRITE,
                            MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
}

/* The policy of the page at `addr` as get_mempolicy(MPOL_F_ADDR) reports
 * it: the mode (with its mode flags) and the first word of the nodemask. */
static A_UNUSED u64 pol_nodes_buf[16];
static A_UNUSED i64 pol_mode(void *addr) {
    int mode = -1;
    memset(pol_nodes_buf, 0xff, sizeof pol_nodes_buf);
    i64 r = syscall5(SYS_get_mempolicy, &mode, pol_nodes_buf, 1025, addr, MPOL_F_ADDR);
    return r < 0 ? r : mode;
}
static A_UNUSED u64 pol_nodes(void *addr) {
    pol_mode(addr);
    return pol_nodes_buf[0];
}

int test_main(void) {
    u64 node0 = 1, node1 = 2, none = 0;
    char *p = region(4);
    expect("len_1", mbind(p, 1, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_page_plus_1", mbind(p, PAGE + 1, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_whole", mbind(p, 4 * PAGE, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_rounds_into_guard", mbind(p, 4 * PAGE + 1, MPOL_BIND, &node0, 65, 0), 0);
    expect("unaligned_addr", mbind(p + 1, PAGE, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("unaligned_addr_2048", mbind(p + 2048, PAGE, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("unaligned_addr_len_0", mbind(p + 1, 0, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("len_0", mbind(p, 0, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_0_addr_0", mbind(0, 0, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_0_empty_nodes", mbind(p, 0, MPOL_BIND, &none, 65, 0), 0);
    expect("len_0_offline_node", mbind(p, 0, MPOL_BIND, &node1, 65, 0), 0);
    expect("len_0_default_with_nodes", mbind(p, 0, MPOL_DEFAULT, &node0, 65, 0), 0);
    expect("len_0_local_with_nodes", mbind(p, 0, MPOL_LOCAL, &node0, 65, 0), 0);
    expect("len_0_local_static", mbind(p, 0, MPOL_LOCAL | MPOL_F_STATIC_NODES, 0, 0, 0), 0);
    expect("len_0_unmapped", mbind((void *)0x10000, 0, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_max_rounds_to_0", mbind(p, (u64)-1, MPOL_BIND, &node0, 65, 0), 0);
    expect("len_max_minus_4094", mbind(p, (u64)-4095, MPOL_BIND, &none, 65, 0), 0);
    expect("len_wraps", mbind(p, (u64)-4096, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("len_wraps_default", mbind(p, (u64)-4096, MPOL_DEFAULT, 0, 0, 0), -EINVAL);
    expect("top_page_wraps", mbind((void *)0xfffffffffffff000ULL, PAGE, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("top_page_len_4095", mbind((void *)0xfffffffffffff000ULL, 4095, MPOL_BIND, &node0, 65, 0), -EINVAL);
    expect("below_top_page", mbind((void *)0xffffffffffffe000ULL, PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("addr_0", mbind(0, PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("noncanonical", mbind((void *)0x800000000000ULL, PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("kernel_half", mbind((void *)0xffffffff80000000ULL, PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("unmapped_page", (syscall2(SYS_munmap, p + 2 * PAGE, PAGE),
                             mbind(p + 2 * PAGE, PAGE, MPOL_BIND, &node0, 65, 0)), -EFAULT);
    expect("ends_in_unmapped", mbind(p + PAGE, 2 * PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("starts_in_unmapped", mbind(p + 2 * PAGE, 2 * PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("hole_in_middle", mbind(p, 4 * PAGE, MPOL_BIND, &node0, 65, 0), -EFAULT);
    expect("hole_preferred", mbind(p, 4 * PAGE, MPOL_PREFERRED, &node0, 65, 0), -EFAULT);
    expect("hole_local", mbind(p, 4 * PAGE, MPOL_LOCAL, 0, 0, 0), -EFAULT);
    expect("mapped_part_ok", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0), 0);
    return finish();
}
