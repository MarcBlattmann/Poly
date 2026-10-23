/* What the nodemask must hold, per mode, on a machine with one memory
 * node (node 0): MPOL_DEFAULT needs the empty set, MPOL_LOCAL too;
 * MPOL_PREFERRED takes the empty set or nodes; MPOL_BIND, INTERLEAVE,
 * PREFERRED_MANY and WEIGHTED_INTERLEAVE need a non-empty set. A set with
 * no online node (node 1, 2, 63, 512) is EINVAL, a set with node 0 and
 * offline nodes is fine. MPOL_F_STATIC_NODES needs an online node too;
 * MPOL_F_RELATIVE_NODES takes any non-empty set.
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
    u64 none = 0, node0 = 1, node1 = 2, nodes01 = 3, node2 = 4, node63 = 1ULL << 63;
    u64 node512[16] = { 0 };
    node512[8] = 1;
    char *p = region(1);
    expect("default_node0", mbind(p, PAGE, MPOL_DEFAULT, &node0, 65, 0), -EINVAL);
    expect("default_node1", mbind(p, PAGE, MPOL_DEFAULT, &node1, 65, 0), -EINVAL);
    expect("default_static_node0", mbind(p, PAGE, MPOL_DEFAULT | MPOL_F_STATIC_NODES, &node0, 65, 0), -EINVAL);
    expect("local_node0", mbind(p, PAGE, MPOL_LOCAL, &node0, 65, 0), -EINVAL);
    expect("local_empty_mask", mbind(p, PAGE, MPOL_LOCAL, &none, 65, 0), 0);
    expect("preferred_empty_mask", mbind(p, PAGE, MPOL_PREFERRED, &none, 65, 0), 0);
    expect("preferred_node1", mbind(p, PAGE, MPOL_PREFERRED, &node1, 65, 0), -EINVAL);
    expect("preferred_nodes01", mbind(p, PAGE, MPOL_PREFERRED, &nodes01, 65, 0), 0);
    expect("bind_empty_mask", mbind(p, PAGE, MPOL_BIND, &none, 65, 0), -EINVAL);
    expect("bind_null", mbind(p, PAGE, MPOL_BIND, 0, 65, 0), -EINVAL);
    expect("bind_maxnode_1", mbind(p, PAGE, MPOL_BIND, &node0, 1, 0), -EINVAL);
    expect("interleave_empty", mbind(p, PAGE, MPOL_INTERLEAVE, 0, 0, 0), -EINVAL);
    expect("preferred_many_empty", mbind(p, PAGE, MPOL_PREFERRED_MANY, 0, 0, 0), -EINVAL);
    expect("weighted_empty", mbind(p, PAGE, MPOL_WEIGHTED_INTERLEAVE, 0, 0, 0), -EINVAL);
    expect("bind_node1", mbind(p, PAGE, MPOL_BIND, &node1, 65, 0), -EINVAL);
    expect("bind_node2", mbind(p, PAGE, MPOL_BIND, &node2, 65, 0), -EINVAL);
    expect("bind_node63", mbind(p, PAGE, MPOL_BIND, &node63, 65, 0), -EINVAL);
    expect("bind_node512", mbind(p, PAGE, MPOL_BIND, node512, 1025, 0), -EINVAL);
    expect("interleave_node1", mbind(p, PAGE, MPOL_INTERLEAVE, &node1, 65, 0), -EINVAL);
    expect("preferred_many_node1", mbind(p, PAGE, MPOL_PREFERRED_MANY, &node1, 65, 0), -EINVAL);
    expect("weighted_node1", mbind(p, PAGE, MPOL_WEIGHTED_INTERLEAVE, &node1, 65, 0), -EINVAL);
    expect("bind_nodes01", mbind(p, PAGE, MPOL_BIND, &nodes01, 65, 0), 0);
    expect("interleave_nodes01", mbind(p, PAGE, MPOL_INTERLEAVE, &nodes01, 65, 0), 0);
    expect("bind_static_node1", mbind(p, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES, &node1, 65, 0), -EINVAL);
    expect("preferred_static_node2", mbind(p, PAGE, MPOL_PREFERRED | MPOL_F_STATIC_NODES, &node2, 65, 0), -EINVAL);
    expect("bind_static_nodes01", mbind(p, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES, &nodes01, 65, 0), 0);
    expect("bind_relative_node1", mbind(p, PAGE, MPOL_BIND | MPOL_F_RELATIVE_NODES, &node1, 65, 0), 0);
    expect("interleave_relative_node63",
           mbind(p, PAGE, MPOL_INTERLEAVE | MPOL_F_RELATIVE_NODES, &node63, 65, 0), 0);
    expect("preferred_relative_node2", mbind(p, PAGE, MPOL_PREFERRED | MPOL_F_RELATIVE_NODES, &node2, 65, 0), 0);
    expect("bind_relative_node512", mbind(p, PAGE, MPOL_BIND | MPOL_F_RELATIVE_NODES, node512, 1025, 0), 0);
    expect("bind_relative_empty", mbind(p, PAGE, MPOL_BIND | MPOL_F_RELATIVE_NODES, &none, 65, 0), -EINVAL);
    return finish();
}
