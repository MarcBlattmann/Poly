/* maxnode counts one more than the number of nodemask bits used: bits 0
 * to maxnode-2 are read, so maxnode 1 is the empty set and maxnode 2 uses
 * node 0 only. A NULL nodemask is the empty set whatever maxnode is. A
 * non-NULL nodemask with maxnode 0, or maxnode above 32769 (one page of
 * bits plus one), is EINVAL; 32769 itself is accepted.
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
    u64 node0 = 1, node1 = 2, nodes01 = 3, none = 0;
    char *p = region(1);
    char *big = (char *)syscall6(SYS_mmap, 0, 2 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    big[0] = 1; /* node 0 */
    expect("maxnode_2_node0", mbind(p, PAGE, MPOL_BIND, &node0, 2, 0), 0);
    expect("maxnode_1_is_empty", mbind(p, PAGE, MPOL_BIND, &node0, 1, 0), -EINVAL);
    expect("maxnode_1_default_ok", mbind(p, PAGE, MPOL_DEFAULT, &node0, 1, 0), 0);
    expect("maxnode_1_preferred_ok", mbind(p, PAGE, MPOL_PREFERRED, &node0, 1, 0), 0);
    expect("maxnode_2_ignores_node1", mbind(p, PAGE, MPOL_BIND, &node1, 2, 0), -EINVAL);
    expect("maxnode_2_nodes01", mbind(p, PAGE, MPOL_BIND, &nodes01, 2, 0), 0);
    expect("maxnode_3_reads_node1", mbind(p, PAGE, MPOL_BIND, &node1, 3, 0), -EINVAL);
    expect("maxnode_2_default_ignores_node1", mbind(p, PAGE, MPOL_DEFAULT, &node1, 2, 0), 0);
    expect("maxnode_3_default_sees_node1", mbind(p, PAGE, MPOL_DEFAULT, &node1, 3, 0), -EINVAL);
    expect("maxnode_0_nonnull_default", mbind(p, PAGE, MPOL_DEFAULT, &none, 0, 0), -EINVAL);
    expect("maxnode_0_nonnull_bind", mbind(p, PAGE, MPOL_BIND, &node0, 0, 0), -EINVAL);
    expect("maxnode_0_null_default", mbind(p, PAGE, MPOL_DEFAULT, 0, 0, 0), 0);
    expect("maxnode_huge_null_default", mbind(p, PAGE, MPOL_DEFAULT, 0, 1ULL << 40, 0), 0);
    expect("maxnode_max_null_preferred", mbind(p, PAGE, MPOL_PREFERRED, 0, (u64)-1, 0), 0);
    expect("maxnode_1025", mbind(p, PAGE, MPOL_BIND, big, 1025, 0), 0);
    expect("maxnode_4096", mbind(p, PAGE, MPOL_BIND, big, 4096, 0), 0);
    expect("maxnode_32768", mbind(p, PAGE, MPOL_BIND, big, 32768, 0), 0);
    expect("maxnode_32769", mbind(p, PAGE, MPOL_BIND, big, 32769, 0), 0);
    expect("maxnode_32770", mbind(p, PAGE, MPOL_BIND, big, 32770, 0), -EINVAL);
    expect("maxnode_65536", mbind(p, PAGE, MPOL_BIND, big, 65536, 0), -EINVAL);
    expect("maxnode_2_pow_40", mbind(p, PAGE, MPOL_BIND, big, 1ULL << 40, 0), -EINVAL);
    expect("maxnode_max", mbind(p, PAGE, MPOL_BIND, big, (u64)-1, 0), -EINVAL);
    expect("maxnode_32770_default", mbind(p, PAGE, MPOL_DEFAULT, big + PAGE, 32770, 0), -EINVAL);
    return finish();
}
