/* The policy mbind sets, as get_mempolicy(MPOL_F_ADDR) reports it for a
 * page of the range: the mode with its mode flags; for BIND, INTERLEAVE,
 * PREFERRED_MANY and WEIGHTED_INTERLEAVE the given nodes that are online
 * (node 0 here), for PREFERRED the first such node, and with
 * MPOL_F_STATIC_NODES or MPOL_F_RELATIVE_NODES the nodemask exactly as
 * given. MPOL_PREFERRED with no nodes reads back as MPOL_LOCAL. Pages
 * outside the range keep their policy; a fresh mapping is MPOL_DEFAULT.
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
    u64 node0 = 1, nodes01 = 3, node1 = 2;
    char *p = region(12);
    expect("fresh_default", pol_mode(p), MPOL_DEFAULT);
    expect("fresh_no_nodes", pol_nodes(p), 0);
    mbind(p, PAGE, MPOL_BIND, &nodes01, 65, 0);
    expect("bind_mode", pol_mode(p), MPOL_BIND);
    expect("bind_online_nodes", pol_nodes(p), 1);
    mbind(p + PAGE, PAGE, MPOL_INTERLEAVE, &nodes01, 65, 0);
    expect("interleave_mode", pol_mode(p + PAGE), MPOL_INTERLEAVE);
    expect("interleave_nodes", pol_nodes(p + PAGE), 1);
    mbind(p + 2 * PAGE, PAGE, MPOL_PREFERRED, &nodes01, 65, 0);
    expect("preferred_mode", pol_mode(p + 2 * PAGE), MPOL_PREFERRED);
    expect("preferred_nodes", pol_nodes(p + 2 * PAGE), 1);
    mbind(p + 3 * PAGE, PAGE, MPOL_PREFERRED, 0, 0, 0);
    expect("preferred_empty_is_local", pol_mode(p + 3 * PAGE), MPOL_LOCAL);
    expect("preferred_empty_nodes", pol_nodes(p + 3 * PAGE), 0);
    mbind(p + 4 * PAGE, PAGE, MPOL_LOCAL, 0, 0, 0);
    expect("local_mode", pol_mode(p + 4 * PAGE), MPOL_LOCAL);
    expect("local_nodes", pol_nodes(p + 4 * PAGE), 0);
    mbind(p + 5 * PAGE, PAGE, MPOL_PREFERRED_MANY, &nodes01, 65, 0);
    expect("preferred_many_mode", pol_mode(p + 5 * PAGE), MPOL_PREFERRED_MANY);
    expect("preferred_many_nodes", pol_nodes(p + 5 * PAGE), 1);
    mbind(p + 6 * PAGE, PAGE, MPOL_WEIGHTED_INTERLEAVE, &nodes01, 65, 0);
    expect("weighted_mode", pol_mode(p + 6 * PAGE), MPOL_WEIGHTED_INTERLEAVE);
    expect("weighted_nodes", pol_nodes(p + 6 * PAGE), 1);
    mbind(p + 7 * PAGE, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES, &nodes01, 65, 0);
    expect("static_mode", pol_mode(p + 7 * PAGE), MPOL_BIND | MPOL_F_STATIC_NODES);
    expect("static_nodes_as_given", pol_nodes(p + 7 * PAGE), 3);
    mbind(p + 8 * PAGE, PAGE, MPOL_INTERLEAVE | MPOL_F_RELATIVE_NODES, &node1, 65, 0);
    expect("relative_mode", pol_mode(p + 8 * PAGE), MPOL_INTERLEAVE | MPOL_F_RELATIVE_NODES);
    expect("relative_nodes_as_given", pol_nodes(p + 8 * PAGE), 2);
    mbind(p + 9 * PAGE, PAGE, MPOL_BIND | MPOL_F_NUMA_BALANCING, &node0, 65, 0);
    expect("balancing_mode", pol_mode(p + 9 * PAGE), MPOL_BIND | MPOL_F_NUMA_BALANCING);
    mbind(p + 10 * PAGE, PAGE, MPOL_PREFERRED | MPOL_F_STATIC_NODES, &nodes01, 65, 0);
    expect("preferred_static_mode", pol_mode(p + 10 * PAGE), MPOL_PREFERRED | MPOL_F_STATIC_NODES);
    expect("preferred_static_nodes", pol_nodes(p + 10 * PAGE), 3);
    expect("untouched_page", pol_mode(p + 11 * PAGE), MPOL_DEFAULT);
    expect("guard_below", pol_mode(p - PAGE), MPOL_DEFAULT);
    /* A later call replaces the policy of part of a range only. */
    expect("interleave_middle", mbind(p + 4 * PAGE, 2 * PAGE, MPOL_INTERLEAVE, &node0, 65, 0), 0);
    expect("replaced_local", pol_mode(p + 4 * PAGE), MPOL_INTERLEAVE);
    expect("replaced_preferred_many", pol_mode(p + 5 * PAGE), MPOL_INTERLEAVE);
    expect("neighbour_kept", pol_mode(p + 3 * PAGE), MPOL_LOCAL);
    expect("neighbour_after_kept", pol_mode(p + 6 * PAGE), MPOL_WEIGHTED_INTERLEAVE);
    expect("default_all", mbind(p, 12 * PAGE, MPOL_DEFAULT, 0, 0, 0), 0);
    expect("all_default_first", pol_mode(p), MPOL_DEFAULT);
    expect("all_default_last", pol_mode(p + 10 * PAGE), MPOL_DEFAULT);
    expect("default_nodes_empty", pol_nodes(p + 7 * PAGE), 0);
    /* A failed call changes nothing. */
    mbind(p, PAGE, MPOL_BIND, &node0, 65, 0);
    expect("failed_offline", mbind(p, PAGE, MPOL_INTERLEAVE, &node1, 65, 0), -EINVAL);
    expect("failed_kept_bind", pol_mode(p), MPOL_BIND);
    /* Unmapping and mapping again gives a fresh (default) policy. */
    syscall2(SYS_munmap, p, PAGE);
    syscall6(SYS_mmap, (i64)p, PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    expect("remapped_default", pol_mode(p), MPOL_DEFAULT);
    return finish();
}
