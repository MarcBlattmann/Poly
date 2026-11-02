/* get_mempolicy(2) reports the mode with its mode flags ORed in
 * (MPOL_F_STATIC_NODES 0x8000, MPOL_F_RELATIVE_NODES 0x4000,
 * MPOL_F_NUMA_BALANCING 0x2000). With STATIC or RELATIVE the mask
 * reported is the one the caller gave, also bits of nodes that do not
 * exist; without them it is the effective mask (only existing nodes), and
 * for MPOL_PREFERRED only the first node of the given mask.
 * Assumes one node (node 0): node 1 and up do not exist.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_pread64 17
#define SYS_pwrite64 18
#define SYS_madvise 28
#define SYS_ftruncate 77
#define SYS_geteuid 107
#define SYS_remap_file_pages 216
#define SYS_mbind 237
#define SYS_set_mempolicy 238
#define SYS_get_mempolicy 239
#define SYS_waitid 247
#define SYS_migrate_pages 256
#define SYS_move_pages 279
#define SYS_pidfd_open 434
#define SYS_process_madvise 440
#define SYS_process_mrelease 448
#define SYS_set_mempolicy_home_node 450
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define ENOMEM 12
#define EFAULT 14
#define EINVAL 22
#define EOPNOTSUPP 95
/* linux/mempolicy.h: modes, mode flags, get_mempolicy and mbind flags */
#define MPOL_DEFAULT 0
#define MPOL_PREFERRED 1
#define MPOL_BIND 2
#define MPOL_INTERLEAVE 3
#define MPOL_LOCAL 4
#define MPOL_PREFERRED_MANY 5
#define MPOL_WEIGHTED_INTERLEAVE 6
#define MPOL_F_NUMA_BALANCING (1 << 13)
#define MPOL_F_RELATIVE_NODES (1 << 14)
#define MPOL_F_STATIC_NODES (1 << 15)
#define MPOL_F_NODE (1 << 0)
#define MPOL_F_ADDR (1 << 1)
#define MPOL_F_MEMS_ALLOWED (1 << 2)
#define MPOL_MF_STRICT (1 << 0)
#define MPOL_MF_MOVE (1 << 1)
#define MPOL_MF_MOVE_ALL (1 << 2)
/* asm-generic/mman-common.h */
#define PROT_READ 0x1
#define PROT_WRITE 0x2
#define MAP_POPULATE 0x8000
#define MAP_NONBLOCK 0x10000
#define MADV_WILLNEED 3
#define MADV_DONTNEED 4
#define MADV_COLD 20
#define MADV_PAGEOUT 21
/* linux/wait.h */
#define P_PID 1
#define WEXITED 4
#define WNOWAIT 0x01000000
#define PG 4096L


static __attribute__((unused)) i64 open3(const char *path, i64 flags, i64 mode) {
    return syscall4(SYS_openat, AT_FDCWD, path, flags, mode);
}
static __attribute__((unused)) i64 unlink_path(const char *path) { return syscall3(SYS_unlinkat, AT_FDCWD, path, 0); }
static __attribute__((unused)) char *map_anon(u64 len, int prot, int flags) {
    return (char *)syscall6(SYS_mmap, 0, (i64)len, prot, flags | MAP_ANONYMOUS, -1, 0);
}
static __attribute__((unused)) i64 unmap(void *p, u64 len) { return syscall2(SYS_munmap, p, len); }
static __attribute__((unused)) i64 get_pol(int *mode, u64 *mask, u64 maxnode, void *addr, u64 flags) {
    return syscall5(SYS_get_mempolicy, mode, mask, maxnode, addr, flags);
}
static __attribute__((unused)) i64 set_pol(i64 mode, u64 *mask, u64 maxnode) {
    return syscall3(SYS_set_mempolicy, mode, mask, maxnode);
}
static __attribute__((unused)) i64 mbind(void *addr, u64 len, i64 mode, u64 *mask, u64 maxnode, u64 flags) {
    return syscall6(SYS_mbind, (i64)addr, (i64)len, mode, (i64)mask, (i64)maxnode, (i64)flags);
}
static __attribute__((unused)) i64 move_pages(i64 pid, u64 count, void **pages, int *nodes, int *status, i64 flags) {
    return syscall6(SYS_move_pages, pid, (i64)count, (i64)pages, (i64)nodes, (i64)status, flags);
}
static __attribute__((unused)) i64 euid(void) { return syscall0(SYS_geteuid); }
/* fork(2) as clone(SIGCHLD): the child returns 0. */
static __attribute__((unused)) i64 fork_child(void) { return syscall5(SYS_clone, SIGCHLD, 0, 0, 0, 0); }
static __attribute__((unused)) void child_exit(int code) {
    for (;;) syscall1(SYS_exit_group, code);
}
/* Waits for the child; returns its raw wait status. */
static __attribute__((unused)) int wait_child(i64 pid) {
    int st = -1;
    syscall4(SYS_wait4, pid, &st, 0, 0);
    return st;
}
static __attribute__((unused)) i64 pidfd_open(i64 pid, i64 flags) { return syscall2(SYS_pidfd_open, pid, flags); }
/* A pid that no longer exists: a child that has been reaped. */
static __attribute__((unused)) i64 gone_pid(void) {
    i64 pid = fork_child();
    if (pid == 0) child_exit(0);
    wait_child(pid);
    return pid;
}
static __attribute__((unused)) int drop_to_user(void) {
    if (syscall3(SYS_setresgid, 1000, 1000, 1000) != 0) return 0;
    if (syscall3(SYS_setresuid, 1000, 1000, 1000) != 0) return 0;
    return 1;
}
/* Blocks in the child until the parent kills it. */
static __attribute__((unused)) void child_wait_forever(void) {
    for (;;) syscall0(SYS_pause);
}

static int got(int want_mode, u64 want_mask) {
    int mode = -7;
    u64 nm[2] = { 0xee, 0xee };
    if (get_pol(&mode, nm, 128, 0, 0) != 0) return 0;
    return mode == want_mode && nm[0] == want_mask && nm[1] == 0;
}

int test_main(void) {
    u64 m;
    i64 r;

    m = 3;
    r = set_pol(MPOL_BIND | MPOL_F_STATIC_NODES, &m, 64);
    check("bind_static_0_1", r == 0 && got(MPOL_BIND | MPOL_F_STATIC_NODES, 3), r);
    m = 2;
    r = set_pol(MPOL_BIND | MPOL_F_RELATIVE_NODES, &m, 64);
    check("bind_relative_1", r == 0 && got(MPOL_BIND | MPOL_F_RELATIVE_NODES, 2), r);
    m = 8;
    r = set_pol(MPOL_INTERLEAVE | MPOL_F_RELATIVE_NODES, &m, 64);
    check("interleave_relative_3", r == 0 && got(MPOL_INTERLEAVE | MPOL_F_RELATIVE_NODES, 8), r);
    m = 3;
    r = set_pol(MPOL_PREFERRED | MPOL_F_STATIC_NODES, &m, 64);
    check("preferred_static_0_1", r == 0 && got(MPOL_PREFERRED | MPOL_F_STATIC_NODES, 3), r);
    m = 3;
    r = set_pol(MPOL_PREFERRED, &m, 64);
    check("preferred_first_node_only", r == 0 && got(MPOL_PREFERRED, 1), r);
    m = 3;
    r = set_pol(MPOL_BIND, &m, 64);
    check("bind_effective_mask", r == 0 && got(MPOL_BIND, 1), r);
    m = 3;
    r = set_pol(MPOL_INTERLEAVE, &m, 64);
    check("interleave_effective_mask", r == 0 && got(MPOL_INTERLEAVE, 1), r);
    m = 1;
    r = set_pol(MPOL_BIND | MPOL_F_NUMA_BALANCING, &m, 64);
    check("bind_balancing", r == 0 && got(MPOL_BIND | MPOL_F_NUMA_BALANCING, 1), r);
    r = set_pol(MPOL_DEFAULT, 0, 0);
    check("back_to_default", r == 0 && got(MPOL_DEFAULT, 0), r);
    return finish();
}
