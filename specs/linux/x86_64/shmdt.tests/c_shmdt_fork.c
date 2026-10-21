/* A forked child inherits the parent's attachments (nattch 2). shmdt in
 * the child detaches only the child's copy: nattch drops to 1, the child's
 * address becomes unmapped, the parent's stays mapped and sees what the
 * child wrote before detaching. A touch of the detached address kills the
 * child with SIGSEGV.
 * poly-diff: syscalls=off (two processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
/* linux/ipc.h, linux/shm.h */
#define IPC_PRIVATE 0
#define IPC_RMID 0
#define IPC_STAT 2
#define SHM_RDONLY 010000
#define SHM_RND 020000
#define SHM_REMAP 040000
#define SHM_EXEC 0100000
#define SHM_DEST 01000

/* struct shmid64_ds (asm-generic/shmbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64. */
struct k_shmid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    u64 segsz;
    i64 atime, dtime, ctime;
    int cpid, lpid;
    u64 nattch;
    u64 unused4, unused5;
};

static __attribute__((unused)) i64 shmget(i64 key, u64 size, i64 flg) { return syscall3(SYS_shmget, key, size, flg); }
static __attribute__((unused)) i64 shmat(i64 id, u64 addr, i64 flg) { return syscall3(SYS_shmat, id, addr, flg); }
static __attribute__((unused)) i64 shmdt(i64 addr) { return syscall1(SYS_shmdt, addr); }
static __attribute__((unused)) i64 shm_rm(i64 id) { return syscall3(SYS_shmctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_shmctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) int is_root(void) { return syscall0(SYS_geteuid) == 0; }
#define SYS_mincore 27
#define IPC_SET 1
/* Whether the page at `addr` is mapped (mincore succeeds) right now. */
static __attribute__((unused)) int mapped(u64 addr) {
    unsigned char vec[1];
    return syscall3(SYS_mincore, addr, 4096, vec) == 0;
}
static __attribute__((unused)) i64 nattch(i64 id) {
    struct k_shmid_ds ds;
    i64 r = shm_stat(id, &ds);
    return r < 0 ? r : (i64)ds.nattch;
}
/* The start of `len` bytes of address space that are free right now. */
static __attribute__((unused)) u64 free_range(u64 len) {
    i64 r = syscall6(SYS_mmap, 0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, r, len);
    return (u64)r;
}
int test_main(void) {
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    i64 a = shmat(id, 0, 0);
    check("attach_ok", a > 0, a);
    volatile unsigned *res = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        unsigned bits = 0;
        if (nattch(id) == 2) bits |= 1;
        ((volatile char *)a)[0] = 'c';
        if (shmdt(a) == 0) bits |= 2;
        if (!mapped(a)) bits |= 4;
        if (nattch(id) == 1) bits |= 8;
        if (shmdt(a) == -EINVAL) bits |= 16;
        res[0] = bits;
        ((volatile char *)a)[0] = 'd';
        exit_group(0);
    }
    int s = wait_status(pid);
    check("child_saw_nattch_2", res[0] & 1, res[0]);
    check("child_shmdt_ok", res[0] & 2, res[0]);
    check("child_unmapped", res[0] & 4, res[0]);
    check("child_saw_nattch_1", res[0] & 8, res[0]);
    check("child_again_einval", res[0] & 16, res[0]);
    check("child_touch_segv", W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV, s);
    check("parent_mapped", mapped(a), 0);
    check("parent_sees_write", ((volatile char *)a)[0] == 'c', ((volatile char *)a)[0]);
    check("nattch_1", nattch(id) == 1, nattch(id));
    i64 r = shmdt(a);
    check("parent_shmdt_ok", r == 0, r);
    shm_rm(id);
    return finish();
}
