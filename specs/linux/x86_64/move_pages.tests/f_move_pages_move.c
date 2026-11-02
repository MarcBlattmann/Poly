/* move_pages(2) with a nodes array moves pages[i] to nodes[i] and puts
 * the resulting node or -errno into status[i]: to node 0 a populated page
 * reports 0, an unpopulated one -ENOENT, an unmapped address -EFAULT;
 * the call returns 0 (pages that could not be moved are not counted as
 * failures of the call). A target node that does not exist (1, 5000, -1)
 * fails the whole call with ENODEV (19), status untouched. flags: only
 * MPOL_MF_MOVE (2) and MPOL_MF_MOVE_ALL (4); MOVE_ALL needs CAP_SYS_NICE
 * (EPERM otherwise); MPOL_MF_STRICT (1) or any other bit: EINVAL.
 * Assumes one node (node 0).
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

int test_main(void) {
    char *m = map_anon(3 * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE);
    m[0] = 'x';
    void *pages[3] = { m, m + PG, (void *)0x1000 };
    int nodes[3] = { 0, 0, 0 };
    int st[3];
    i64 r;

    memset(st, 0x7f, sizeof st);
    r = move_pages(0, 3, pages, nodes, st, 0);
    check("move_to_node0", r == 0 && st[0] == 0 && st[1] == -ENOENT && st[2] == -EFAULT, r);
    check("data_kept", m[0] == 'x', m[0]);

    int bad[3] = { 1, 5000, -1 };
    const char *names[3] = { "node1_enodev", "node5000_enodev", "node_minus1_enodev" };
    for (int i = 0; i < 3; i++) {
        int one_node = bad[i];
        memset(st, 0x7f, sizeof st);
        r = move_pages(0, 1, pages, &one_node, st, 0);
        check(names[i], r == -ENODEV && st[0] == 0x7f7f7f7f, r);
    }
    int mixed[2] = { 0, 1 };
    memset(st, 0x7f, sizeof st);
    r = move_pages(0, 2, pages, mixed, st, 0);
    check("one_bad_node_fails_call", r == -ENODEV, r);

    r = move_pages(0, 1, pages, nodes, st, MPOL_MF_MOVE);
    check("flag_move_ok", r == 0 && st[0] == 0, r);
    r = move_pages(0, 1, pages, nodes, st, MPOL_MF_MOVE_ALL);
    if (euid() == 0)
        check("move_all_root_ok_user_eperm", r == 0, r);
    else
        check("move_all_root_ok_user_eperm", r == -EPERM, r);
    r = move_pages(0, 1, pages, 0, st, MPOL_MF_STRICT);
    check("flag_strict_einval", r == -EINVAL, r);
    r = move_pages(0, 1, pages, 0, st, 8);
    check("flag8_einval", r == -EINVAL, r);
    return finish();
}
