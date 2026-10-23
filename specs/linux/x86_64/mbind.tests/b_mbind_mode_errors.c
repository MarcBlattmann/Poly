/* Invalid modes are EINVAL: a mode number of 7 or more, a mode bit other
 * than MPOL_F_STATIC_NODES, MPOL_F_RELATIVE_NODES and
 * MPOL_F_NUMA_BALANCING, STATIC and RELATIVE together, NUMA_BALANCING
 * with any mode but MPOL_BIND and MPOL_PREFERRED_MANY, and a STATIC or
 * RELATIVE flag on MPOL_LOCAL or on MPOL_PREFERRED with no nodes.
 * Nothing changes on failure.
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
    u64 node0 = 1;
    char *p = region(1);
    expect("mode_7", mbind(p, PAGE, 7, &node0, 65, 0), -EINVAL);
    expect("mode_8", mbind(p, PAGE, 8, &node0, 65, 0), -EINVAL);
    expect("mode_100", mbind(p, PAGE, 100, &node0, 65, 0), -EINVAL);
    expect("mode_minus_1", mbind(p, PAGE, -1, &node0, 65, 0), -EINVAL);
    expect("mode_int_min", mbind(p, PAGE, 0x80000002LL, &node0, 65, 0), -EINVAL);
    expect("mode_bit_12", mbind(p, PAGE, MPOL_BIND | (1 << 12), &node0, 65, 0), -EINVAL);
    expect("mode_bit_8", mbind(p, PAGE, MPOL_BIND | (1 << 8), &node0, 65, 0), -EINVAL);
    expect("mode_bit_16", mbind(p, PAGE, MPOL_BIND | (1 << 16), &node0, 65, 0), -EINVAL);
    expect("static_number_7", mbind(p, PAGE, MPOL_F_STATIC_NODES | 7, &node0, 65, 0), -EINVAL);
    expect("static_and_relative",
           mbind(p, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES, &node0, 65, 0), -EINVAL);
    expect("default_static_and_relative",
           mbind(p, PAGE, MPOL_DEFAULT | MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES, 0, 0, 0), -EINVAL);
    expect("balancing_preferred", mbind(p, PAGE, MPOL_PREFERRED | MPOL_F_NUMA_BALANCING, &node0, 65, 0), -EINVAL);
    expect("balancing_interleave", mbind(p, PAGE, MPOL_INTERLEAVE | MPOL_F_NUMA_BALANCING, &node0, 65, 0), -EINVAL);
    expect("balancing_weighted",
           mbind(p, PAGE, MPOL_WEIGHTED_INTERLEAVE | MPOL_F_NUMA_BALANCING, &node0, 65, 0), -EINVAL);
    expect("balancing_default", mbind(p, PAGE, MPOL_DEFAULT | MPOL_F_NUMA_BALANCING, 0, 0, 0), -EINVAL);
    expect("balancing_local", mbind(p, PAGE, MPOL_LOCAL | MPOL_F_NUMA_BALANCING, 0, 0, 0), -EINVAL);
    expect("balancing_bind_static",
           mbind(p, PAGE, MPOL_BIND | MPOL_F_NUMA_BALANCING | MPOL_F_STATIC_NODES, &node0, 65, 0), 0);
    expect("local_static", mbind(p, PAGE, MPOL_LOCAL | MPOL_F_STATIC_NODES, 0, 0, 0), -EINVAL);
    expect("local_relative", mbind(p, PAGE, MPOL_LOCAL | MPOL_F_RELATIVE_NODES, 0, 0, 0), -EINVAL);
    expect("preferred_empty_static", mbind(p, PAGE, MPOL_PREFERRED | MPOL_F_STATIC_NODES, 0, 0, 0), -EINVAL);
    expect("preferred_empty_relative", mbind(p, PAGE, MPOL_PREFERRED | MPOL_F_RELATIVE_NODES, 0, 0, 0), -EINVAL);
    expect("default_relative_nonempty", mbind(p, PAGE, MPOL_DEFAULT | MPOL_F_RELATIVE_NODES, &node0, 65, 0), -EINVAL);
    /* A bad mode is reported even for an empty or unaligned range. */
    expect("bad_mode_len_0", mbind(p, 0, 9, &node0, 65, 0), -EINVAL);
    expect("bad_mode_unaligned_len_0", mbind(p + 1, 0, 9, 0, 0, 0), -EINVAL);
    /* Earlier set policy: interleave; the failures leave it. */
    expect("set_interleave", mbind(p, PAGE, MPOL_INTERLEAVE, &node0, 65, 0), 0);
    expect("bad_mode_again", mbind(p, PAGE, 7, &node0, 65, 0), -EINVAL);
    expect("policy_unchanged", pol_mode(p), MPOL_INTERLEAVE);
    return finish();
}
