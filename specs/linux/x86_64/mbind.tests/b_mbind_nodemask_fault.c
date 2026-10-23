/* The nodemask is read in whole 8-byte words: ceil((maxnode-1)/64) of
 * them. Any unreadable byte of those words is EFAULT (a 4-byte mask
 * before an unmapped page is EFAULT even with maxnode 2); words past them
 * are never read. maxnode 1 or a NULL nodemask reads nothing. The read
 * happens even for an empty range and before the move-all permission
 * check, but after the mode check.
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
    char *p = region(1);
    char *m = region(1); /* readable page m, PROT_NONE page at m + PAGE */
    u64 *last = (u64 *)(m + PAGE - 8);
    u32 *last4 = (u32 *)(m + PAGE - 4);
    *last = 1;
    *last4 = 1;
    expect("one_word_maxnode_2", (*last = 1, mbind(p, PAGE, MPOL_BIND, last, 2, 0)), 0);
    expect("one_word_maxnode_65", mbind(p, PAGE, MPOL_BIND, last, 65, 0), 0);
    expect("two_words_maxnode_66", mbind(p, PAGE, MPOL_BIND, last, 66, 0), -EFAULT);
    expect("two_words_maxnode_129", mbind(p, PAGE, MPOL_BIND, last, 129, 0), -EFAULT);
    expect("many_words_maxnode_1025", mbind(p, PAGE, MPOL_BIND, last, 1025, 0), -EFAULT);
    expect("half_word_maxnode_2", mbind(p, PAGE, MPOL_BIND, last4, 2, 0), -EFAULT);
    expect("prot_none_mask", mbind(p, PAGE, MPOL_BIND, m + PAGE, 65, 0), -EFAULT);
    expect("low_address_mask", mbind(p, PAGE, MPOL_BIND, (void *)8, 65, 0), -EFAULT);
    expect("noncanonical_mask", mbind(p, PAGE, MPOL_BIND, (void *)0x800000000000ULL, 65, 0), -EFAULT);
    expect("kernel_address_mask", mbind(p, PAGE, MPOL_BIND, (void *)0xffffffff80000000ULL, 65, 0), -EFAULT);
    expect("default_bad_mask", mbind(p, PAGE, MPOL_DEFAULT, (void *)8, 65, 0), -EFAULT);
    expect("maxnode_1_not_read", mbind(p, PAGE, MPOL_DEFAULT, (void *)8, 1, 0), 0);
    expect("maxnode_1_bind_empty", mbind(p, PAGE, MPOL_BIND, (void *)8, 1, 0), -EINVAL);
    expect("len_0_bad_mask", mbind(p, 0, MPOL_BIND, (void *)8, 65, 0), -EFAULT);
    expect("unaligned_bad_mask", mbind(p + 1, PAGE, MPOL_BIND, (void *)8, 65, 0), -EFAULT);
    expect("move_all_bad_mask", mbind(p, PAGE, MPOL_BIND, (void *)8, 65, MPOL_MF_MOVE_ALL), -EFAULT);
    expect("bad_flags_bad_mask", mbind(p, PAGE, MPOL_BIND, (void *)8, 65, 8), -EFAULT);
    expect("bad_mode_bad_mask", mbind(p, PAGE, 7, (void *)8, 65, 0), -EINVAL);
    expect("bad_mode_flags_bad_mask",
           mbind(p, PAGE, MPOL_BIND | MPOL_F_STATIC_NODES | MPOL_F_RELATIVE_NODES, (void *)8, 65, 0), -EINVAL);
    expect("maxnode_too_big_bad_mask", mbind(p, PAGE, MPOL_BIND, (void *)8, 1 << 20, 0), -EINVAL);
    expect("maxnode_0_bad_mask", mbind(p, PAGE, MPOL_BIND, (void *)8, 0, 0), -EINVAL);
    expect("read_only_mask", (syscall3(SYS_mprotect, m, PAGE, PROT_READ),
                              mbind(p, PAGE, MPOL_BIND, last, 65, 0)), 0);
    return finish();
}
