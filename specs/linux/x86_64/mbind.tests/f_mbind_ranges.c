/* mbind(2) sets the policy of a page range of the address space, read
 * back per page with get_mempolicy(MPOL_F_ADDR). A range in the middle of
 * a mapping splits it: pages outside keep their policy. start must be
 * page aligned (else EINVAL); len is rounded up to whole pages (len 1
 * covers one page), len 0 succeeds and changes nothing. MPOL_DEFAULT
 * (no mask) removes the range's policy. Shared anonymous and PROT_NONE
 * mappings take a policy too. Mode flags are kept and read back.
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

static int mode_at(char *a) {
    int mode = -7;
    if (get_pol(&mode, 0, 0, a, MPOL_F_ADDR) != 0) return -100;
    return mode;
}

int test_main(void) {
    u64 one = 1;
    i64 r;
    char *p = map_anon(6 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE);

    r = mbind(p, 5 * PG, MPOL_BIND, &one, 64, 0);
    check("bind_5_pages", r == 0, r);
    r = mbind(p + 2 * PG, PG, MPOL_INTERLEAVE, &one, 64, 0);
    check("interleave_middle_page", r == 0, r);
    int want[6] = { MPOL_BIND, MPOL_BIND, MPOL_INTERLEAVE, MPOL_BIND, MPOL_BIND, MPOL_DEFAULT };
    int ok = 1;
    for (int i = 0; i < 6; i++)
        if (mode_at(p + i * PG) != want[i]) ok = 0;
    check("split_policies", ok, 0);
    check("last_byte_of_range", mode_at(p + 5 * PG - 1) == MPOL_BIND, mode_at(p + 5 * PG - 1));

    r = mbind(p + 5 * PG, 1, MPOL_PREFERRED, &one, 64, 0);
    check("len1_one_page", r == 0 && mode_at(p + 5 * PG + PG - 1) == MPOL_PREFERRED, r);
    r = mbind(p + 4 * PG, 0, MPOL_INTERLEAVE, &one, 64, 0);
    check("len0_noop", r == 0 && mode_at(p + 4 * PG) == MPOL_BIND, r);
    r = mbind(p + 1, PG, MPOL_BIND, &one, 64, 0);
    check("unaligned_start_einval", r == -EINVAL, r);

    r = mbind(p, 2 * PG, MPOL_DEFAULT, 0, 0, 0);
    check("default_clears", r == 0 && mode_at(p) == MPOL_DEFAULT && mode_at(p + PG) == MPOL_DEFAULT, r);
    r = mbind(p, PG, MPOL_LOCAL, 0, 0, 0);
    check("local", r == 0 && mode_at(p) == MPOL_LOCAL, r);
    r = mbind(p, PG, MPOL_PREFERRED, 0, 0, 0);
    check("preferred_nomask_local", r == 0 && mode_at(p) == MPOL_LOCAL, r);
    r = mbind(p, PG, MPOL_BIND | MPOL_F_STATIC_NODES, &one, 64, 0);
    check("bind_static_kept", r == 0 && mode_at(p) == (MPOL_BIND | MPOL_F_STATIC_NODES), mode_at(p));
    r = mbind(p, PG, MPOL_BIND | MPOL_F_NUMA_BALANCING, &one, 64, 0);
    check("bind_balancing_kept", r == 0 && mode_at(p) == (MPOL_BIND | MPOL_F_NUMA_BALANCING), mode_at(p));

    char *s = map_anon(PG, PROT_READ | PROT_WRITE, MAP_SHARED);
    r = mbind(s, PG, MPOL_BIND, &one, 64, 0);
    check("shared_anon", r == 0 && mode_at(s) == MPOL_BIND, r);
    char *n = map_anon(PG, 0, MAP_PRIVATE);
    r = mbind(n, PG, MPOL_INTERLEAVE, &one, 64, 0);
    check("prot_none", r == 0 && mode_at(n) == MPOL_INTERLEAVE, r);
    p[0] = 1;
    check("memory_usable", p[0] == 1, 0);
    return finish();
}
