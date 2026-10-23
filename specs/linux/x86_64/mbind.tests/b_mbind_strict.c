/* MPOL_MF_STRICT checks pages that are already there: if one is not on a
 * node of the new policy the call is EIO and nothing changes. On this
 * one-node machine every page is on node 0, so a policy naming node 0
 * passes; MPOL_LOCAL and MPOL_PREFERRED with no nodes name no node, so a
 * written page makes them EIO (unwritten pages, or pages only read, do
 * not); MPOL_F_RELATIVE_NODES checks the nodes as given (a set without
 * node 0 fails). MPOL_DEFAULT is never checked, and with MPOL_MF_MOVE the
 * pages are moved instead, so STRICT|MOVE succeeds. Data stays.
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
    u64 node0 = 1, node1 = 2, nodes01 = 3;
    char *p = region(6);
    expect("untouched_local", mbind(p, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT), 0);
    volatile char c = p[PAGE];
    (void)c;
    expect("read_only_local", mbind(p + PAGE, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT), 0);
    p[2 * PAGE] = 7;
    expect("written_local", mbind(p + 2 * PAGE, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT), -EIO);
    expect("written_local_unchanged", pol_mode(p + 2 * PAGE), MPOL_DEFAULT);
    expect("written_preferred_empty", mbind(p + 2 * PAGE, PAGE, MPOL_PREFERRED, 0, 0, MPOL_MF_STRICT), -EIO);
    expect("range_with_one_written", mbind(p, 4 * PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT), -EIO);
    expect("range_unchanged_first", pol_mode(p), MPOL_LOCAL);
    expect("range_unchanged_last", pol_mode(p + 3 * PAGE), MPOL_DEFAULT);
    expect("written_local_no_strict", mbind(p + 2 * PAGE, PAGE, MPOL_LOCAL, 0, 0, 0), 0);
    expect("written_bind_node0", mbind(p + 2 * PAGE, PAGE, MPOL_BIND, &node0, 65, MPOL_MF_STRICT), 0);
    expect("written_bind_set", pol_mode(p + 2 * PAGE), MPOL_BIND);
    expect("written_interleave", mbind(p + 2 * PAGE, PAGE, MPOL_INTERLEAVE, &nodes01, 65, MPOL_MF_STRICT), 0);
    expect("written_preferred", mbind(p + 2 * PAGE, PAGE, MPOL_PREFERRED, &node0, 65, MPOL_MF_STRICT), 0);
    expect("written_preferred_many", mbind(p + 2 * PAGE, PAGE, MPOL_PREFERRED_MANY, &node0, 65, MPOL_MF_STRICT), 0);
    expect("written_weighted", mbind(p + 2 * PAGE, PAGE, MPOL_WEIGHTED_INTERLEAVE, &node0, 65, MPOL_MF_STRICT), 0);
    expect("written_static", mbind(p + 2 * PAGE, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES, &nodes01, 65, MPOL_MF_STRICT), 0);
    expect("written_relative_node0",
           mbind(p + 2 * PAGE, PAGE, MPOL_BIND | MPOL_F_RELATIVE_NODES, &node0, 65, MPOL_MF_STRICT), 0);
    expect("written_relative_node1",
           mbind(p + 2 * PAGE, PAGE, MPOL_BIND | MPOL_F_RELATIVE_NODES, &node1, 65, MPOL_MF_STRICT), -EIO);
    expect("relative_node1_kept_old", pol_mode(p + 2 * PAGE), MPOL_BIND | MPOL_F_RELATIVE_NODES);
    expect("written_relative_node1_no_strict",
           mbind(p + 2 * PAGE, PAGE, MPOL_PREFERRED | MPOL_F_RELATIVE_NODES, &node1, 65, 0), 0);
    expect("written_default", mbind(p + 2 * PAGE, PAGE, MPOL_DEFAULT, 0, 0, MPOL_MF_STRICT), 0);
    expect("written_local_move", mbind(p + 2 * PAGE, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_MOVE), 0);
    expect("written_local_strict_move",
           mbind(p + 2 * PAGE, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT | MPOL_MF_MOVE), 0);
    expect("strict_move_set", pol_mode(p + 2 * PAGE), MPOL_LOCAL);
    expect("data_kept", p[2 * PAGE], 7);

    /* A written page of a shared anonymous mapping. */
    char *sa = (char *)syscall6(SYS_mmap, 0, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    sa[0] = 1;
    expect("shared_written_local", mbind(sa, PAGE, MPOL_LOCAL, 0, 0, MPOL_MF_STRICT), -EIO);
    expect("shared_written_bind", mbind(sa, PAGE, MPOL_BIND, &node0, 65, MPOL_MF_STRICT), 0);
    return finish();
}
