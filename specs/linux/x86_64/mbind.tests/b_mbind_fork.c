/* A fork child inherits the policies mbind set on its memory; changing
 * them in the child leaves the parent's alone. MPOL_MF_MOVE keeps the
 * data of written pages. Policies are per address range of the process,
 * so a CLONE_VM thread sees them too.
 * poly-diff: syscalls=off (child processes)
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

static char *p;
static volatile i32 done;
static volatile i64 seen = -100;

static int thread_fn(void *arg) {
    (void)arg;
    seen = pol_mode(p + PAGE);
    return 0;
}

int test_main(void) {
    u64 node0 = 1;
    p = region(4);
    for (int i = 0; i < 4 * PAGE; i++) p[i] = (char)i;
    expect("interleave", mbind(p, 2 * PAGE, MPOL_INTERLEAVE, &node0, 65, MPOL_MF_MOVE), 0);
    expect("bind", mbind(p + 2 * PAGE, PAGE, MPOL_BIND, &node0, 65, MPOL_MF_MOVE | MPOL_MF_STRICT), 0);
    int same = 1;
    for (int i = 0; i < 4 * PAGE; i++) same &= p[i] == (char)i;
    check("data_kept_after_move", same, 0);

    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        int code = 0;
        if (pol_mode(p) != MPOL_INTERLEAVE) code |= 1;
        if (pol_mode(p + 2 * PAGE) != MPOL_BIND) code |= 2;
        if (pol_mode(p + 3 * PAGE) != MPOL_DEFAULT) code |= 4;
        if (mbind(p, 4 * PAGE, MPOL_LOCAL, 0, 0, 0) != 0) code |= 8;
        if (pol_mode(p) != MPOL_LOCAL) code |= 16;
        exit_group(code);
    }
    i32 status = -1;
    syscall4(SYS_wait4, pid, &status, 0, 0);
    check("child_inherited_interleave", status >= 0 && !((status >> 8) & 1), status);
    check("child_inherited_bind", status >= 0 && !((status >> 8) & 2), status);
    check("child_untouched_default", status >= 0 && !((status >> 8) & 4), status);
    check("child_changed_own", status >= 0 && !((status >> 8) & 24), status);
    expect("parent_still_interleave", pol_mode(p), MPOL_INTERLEAVE);
    expect("parent_still_bind", pol_mode(p + 2 * PAGE), MPOL_BIND);

    a_spawn(0, &done, thread_fn, 0);
    check("thread_ended", a_spin_until(&done, 0, A_SPIN_LIMIT), done);
    expect("thread_sees_policy", seen, MPOL_INTERLEAVE);
    return finish();
}
