/* A successful shmdt sets the segment's dtime to the current time
 * (seconds, CLOCK_REALTIME) and lpid to the caller's pid and lowers
 * nattch; atime, ctime, segsz, cpid, owner and mode do not change. A
 * failed shmdt changes none of them.
 * poly-diff: syscalls=names (attach addresses and segment ids vary)
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
    i64 id = shmget(IPC_PRIVATE, 4096, 0640);
    struct k_shmid_ds before, after;
    shm_stat(id, &before);
    check("dtime_0_before", before.dtime == 0, before.dtime);
    i64 a = shmat(id, 0, 0);
    i64 b = shmat(id, 0, 0);
    check("attach_ok", a > 0 && b > 0, b);
    shm_stat(id, &before);
    i64 r = shmdt(a + 4096 * 1000);
    shm_stat(id, &after);
    check("failed_shmdt_einval", r == -EINVAL, r);
    check("failed_keeps_nattch", after.nattch == 2, (i64)after.nattch);
    check("failed_keeps_dtime", after.dtime == before.dtime, after.dtime);
    struct k_timespec t0, t1;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t0);
    r = shmdt(a);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t1);
    shm_stat(id, &after);
    check("shmdt_ok", r == 0, r);
    check("nattch_1", after.nattch == 1, (i64)after.nattch);
    check("dtime_now", after.dtime >= t0.tv_sec && after.dtime <= t1.tv_sec, after.dtime - t0.tv_sec);
    check("lpid_self", after.lpid == getpid(), after.lpid);
    check("atime_unchanged", after.atime == before.atime, after.atime - before.atime);
    check("ctime_unchanged", after.ctime == before.ctime, after.ctime - before.ctime);
    check("segsz_unchanged", after.segsz == 4096, (i64)after.segsz);
    check("cpid_unchanged", after.cpid == before.cpid, after.cpid);
    check("mode_unchanged", (after.mode & 0777) == 0640 && after.mode == before.mode, after.mode);
    check("uid_unchanged", after.uid == before.uid && after.cuid == before.cuid, after.uid);
    r = shmdt(b);
    shm_stat(id, &after);
    check("second_ok", r == 0, r);
    check("nattch_0", after.nattch == 0, (i64)after.nattch);
    shm_rm(id);
    return finish();
}
