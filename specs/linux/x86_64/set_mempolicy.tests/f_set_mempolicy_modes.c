/* set_mempolicy(2) per mode, with no mask (NULL, maxnode 0) and with the
 * mask {node 0} (maxnode 2 and 64), checked by reading the policy back:
 * - MPOL_DEFAULT (0): only without a mask (a non-empty mask: EINVAL).
 * - MPOL_PREFERRED (1): with a mask; without one it becomes MPOL_LOCAL.
 * - MPOL_BIND (2), MPOL_INTERLEAVE (3), MPOL_PREFERRED_MANY (5),
 *   MPOL_WEIGHTED_INTERLEAVE (6): need a non-empty mask (else EINVAL).
 * - MPOL_LOCAL (4): only without a mask.
 * - 7 and up, and negative modes: EINVAL.
 * A failed call leaves the previous policy in place.
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

static int mode_now(u64 *mask) {
    int mode = -7;
    *mask = 0xee;
    if (get_pol(&mode, mask, 64, 0, 0) != 0) return -100;
    return mode;
}

int test_main(void) {
    static const char *names[7][3] = {
        { "default_nomask", "default_mask_mx2", "default_mask_mx64" },
        { "preferred_nomask", "preferred_mask_mx2", "preferred_mask_mx64" },
        { "bind_nomask", "bind_mask_mx2", "bind_mask_mx64" },
        { "interleave_nomask", "interleave_mask_mx2", "interleave_mask_mx64" },
        { "local_nomask", "local_mask_mx2", "local_mask_mx64" },
        { "preferred_many_nomask", "preferred_many_mask_mx2", "preferred_many_mask_mx64" },
        { "weighted_interleave_nomask", "weighted_interleave_mask_mx2", "weighted_interleave_mask_mx64" },
    };
    /* expected result; -1 = EINVAL, else the mode read back */
    static const int want[7][3] = {
        { MPOL_DEFAULT, -1, -1 },  { MPOL_LOCAL, MPOL_PREFERRED, MPOL_PREFERRED },
        { -1, MPOL_BIND, MPOL_BIND }, { -1, MPOL_INTERLEAVE, MPOL_INTERLEAVE },
        { MPOL_LOCAL, -1, -1 },    { -1, MPOL_PREFERRED_MANY, MPOL_PREFERRED_MANY },
        { -1, MPOL_WEIGHTED_INTERLEAVE, MPOL_WEIGHTED_INTERLEAVE },
    };
    u64 one = 1, got_mask;
    for (int m = 0; m < 7; m++) {
        for (int k = 0; k < 3; k++) {
            set_pol(MPOL_DEFAULT, 0, 0);
            i64 r = k == 0 ? set_pol(m, 0, 0) : set_pol(m, &one, k == 1 ? 2 : 64);
            int mode = mode_now(&got_mask);
            if (want[m][k] < 0) {
                check(names[m][k], r == -EINVAL && mode == MPOL_DEFAULT, r);
            } else {
                u64 wm = (want[m][k] == MPOL_LOCAL || want[m][k] == MPOL_DEFAULT) ? 0 : 1;
                check(names[m][k], r == 0 && mode == want[m][k] && got_mask == wm, r == 0 ? mode : r);
            }
        }
    }
    set_pol(MPOL_DEFAULT, 0, 0);
    i64 r = set_pol(7, &one, 64);
    check("mode7_einval", r == -EINVAL, r);
    r = set_pol(99, 0, 0);
    check("mode99_einval", r == -EINVAL, r);
    r = set_pol(-1, 0, 0);
    check("mode_minus1_einval", r == -EINVAL, r);

    r = set_pol(MPOL_BIND, &one, 64);
    i64 r2 = set_pol(MPOL_INTERLEAVE, 0, 0);
    check("failed_set_keeps_policy", r == 0 && r2 == -EINVAL && mode_now(&got_mask) == MPOL_BIND, r2);
    set_pol(MPOL_DEFAULT, 0, 0);
    return finish();
}
