/* The flags argument: MPOL_MF_STRICT (1), MPOL_MF_MOVE (2) and any mix of
 * them succeed; MPOL_MF_MOVE_ALL (4) without CAP_SYS_NICE is EPERM; any
 * other bit is EINVAL (checked before EPERM). Only the low 32 bits of
 * flags count. The test drops all capabilities first, so it gives the
 * same results for every user.
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

/* asm/unistd_64.h, linux/capability.h */
#define SYS_capset 126
#define LINUX_CAPABILITY_VERSION_3 0x20080522

static void drop_caps(void) {
    struct { u32 version; i32 pid; } h = { LINUX_CAPABILITY_VERSION_3, 0 };
    u32 d[6] = { 0, 0, 0, 0, 0, 0 };
    expect("capset_drop_all", syscall2(SYS_capset, &h, d), 0);
}

int test_main(void) {
    u64 node0 = 1;
    drop_caps();
    char *p = region(2);
    p[0] = 1;
    expect("flags_0", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0), 0);
    expect("strict", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, MPOL_MF_STRICT), 0);
    expect("move", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, MPOL_MF_MOVE), 0);
    expect("strict_move", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, MPOL_MF_STRICT | MPOL_MF_MOVE), 0);
    expect("move_all", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, MPOL_MF_MOVE_ALL), -EPERM);
    expect("move_all_strict_move", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 7), -EPERM);
    expect("move_all_default", mbind(p, 2 * PAGE, MPOL_DEFAULT, 0, 0, MPOL_MF_MOVE_ALL), -EPERM);
    expect("flag_8", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 8), -EINVAL);
    expect("flag_0x10", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0x10), -EINVAL);
    expect("flag_0x40", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0x40), -EINVAL);
    expect("flag_bit_31", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0x80000000LL), -EINVAL);
    expect("unknown_beats_move_all", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 8 | MPOL_MF_MOVE_ALL), -EINVAL);
    expect("bit_31_beats_move_all", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 0x80000004LL), -EINVAL);
    expect("flag_bit_32_ignored", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, 1LL << 32), 0);
    expect("flag_bit_32_move_all", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, (1LL << 32) | MPOL_MF_MOVE_ALL), -EPERM);
    expect("flag_bits_high_strict", mbind(p, 2 * PAGE, MPOL_BIND, &node0, 65, (0xffffLL << 40) | 1), 0);
    /* Bad flags are reported even for an empty range. */
    expect("bad_flag_len_0", mbind(p, 0, MPOL_BIND, &node0, 65, 8), -EINVAL);
    expect("move_all_len_0", mbind(p, 0, MPOL_BIND, &node0, 65, MPOL_MF_MOVE_ALL), -EPERM);
    expect("data_kept", p[0], 1);
    return finish();
}
