/* Unmapped pages in the range: for every mode but MPOL_DEFAULT the call
 * is EFAULT and changes nothing (also not the mapped pages before the
 * hole). MPOL_DEFAULT (also with a mode flag, STRICT or MOVE) skips the
 * holes and resets the mapped pages, as long as at least one mapped page
 * is in the range; a range with no mapped page at all is EFAULT. The
 * policy is read back with get_mempolicy(MPOL_F_ADDR).
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
    char *p = region(6);
    expect("bind_all", mbind(p, 6 * PAGE, MPOL_BIND, &node0, 65, 0), 0);
    syscall2(SYS_munmap, p + 2 * PAGE, PAGE);
    syscall2(SYS_munmap, p + 4 * PAGE, PAGE);
    expect("interleave_over_hole", mbind(p, 6 * PAGE, MPOL_INTERLEAVE, &node0, 65, 0), -EFAULT);
    expect("before_hole_unchanged", pol_mode(p), MPOL_BIND);
    expect("between_holes_unchanged", pol_mode(p + 3 * PAGE), MPOL_BIND);
    expect("after_hole_unchanged", pol_mode(p + 5 * PAGE), MPOL_BIND);
    expect("local_over_hole", mbind(p, 4 * PAGE, MPOL_LOCAL, 0, 0, 0), -EFAULT);
    expect("still_bind", pol_mode(p + PAGE), MPOL_BIND);
    expect("default_over_holes", mbind(p, 6 * PAGE, MPOL_DEFAULT, 0, 0, 0), 0);
    expect("reset_before_hole", pol_mode(p), MPOL_DEFAULT);
    expect("reset_between_holes", pol_mode(p + 3 * PAGE), MPOL_DEFAULT);
    expect("reset_after_hole", pol_mode(p + 5 * PAGE), MPOL_DEFAULT);
    expect("bind_again", mbind(p + 3 * PAGE, PAGE, MPOL_BIND, &node0, 65, 0), 0);
    expect("default_starting_in_hole", mbind(p + 2 * PAGE, 2 * PAGE, MPOL_DEFAULT, 0, 0, 0), 0);
    expect("reset_from_hole_start", pol_mode(p + 3 * PAGE), MPOL_DEFAULT);
    expect("bind_once_more", mbind(p + 3 * PAGE, PAGE, MPOL_BIND, &node0, 65, 0), 0);
    expect("default_ending_in_hole", mbind(p + 3 * PAGE, 2 * PAGE, MPOL_DEFAULT, 0, 0, 0), 0);
    expect("reset_to_hole_end", pol_mode(p + 3 * PAGE), MPOL_DEFAULT);
    expect("default_static_over_holes", mbind(p, 6 * PAGE, MPOL_DEFAULT | MPOL_F_STATIC_NODES, 0, 0, 0), 0);
    expect("default_strict_move_over_holes",
           mbind(p, 6 * PAGE, MPOL_DEFAULT, 0, 0, MPOL_MF_STRICT | MPOL_MF_MOVE), 0);
    expect("default_only_hole", mbind(p + 2 * PAGE, PAGE, MPOL_DEFAULT, 0, 0, 0), -EFAULT);
    expect("default_unmapped_low", mbind((void *)0x10000, PAGE, MPOL_DEFAULT, 0, 0, 0), -EFAULT);
    expect("default_with_nodes_hole", mbind(p, 6 * PAGE, MPOL_DEFAULT, &node0, 65, 0), -EINVAL);
    expect("offline_node_hole", mbind(p + 2 * PAGE, PAGE, MPOL_BIND, (u64[]){ 2 }, 65, 0), -EINVAL);
    expect("empty_nodes_hole", mbind(p + 2 * PAGE, PAGE, MPOL_BIND, (u64[]){ 0 }, 65, 0), -EINVAL);
    return finish();
}
