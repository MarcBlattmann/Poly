/* A successful attach updates the segment's IPC_STAT record: nattch goes
 * up by one per attachment, lpid becomes the caller's pid and atime the
 * current time in seconds. dtime and ctime do not change. A new segment
 * starts with atime 0, lpid 0, nattch 0.
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
/* The start of `len` bytes of address space that are free right now. */
static __attribute__((unused)) u64 free_range(u64 len) {
    i64 r = syscall6(SYS_mmap, 0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, r, len);
    return (u64)r;
}
static i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
int test_main(void) {
    i64 id = shmget(IPC_PRIVATE, 100, 0600);
    check("create_ok", id >= 0, id);
    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("new_atime_0", ds.atime == 0, ds.atime);
    check("new_lpid_0", ds.lpid == 0, ds.lpid);
    check("new_nattch_0", ds.nattch == 0, (i64)ds.nattch);
    i64 ctime = ds.ctime;

    sleep_ms(1100);
    i64 t0 = now_sec();
    i64 a = shmat(id, 0, 0);
    i64 t1 = now_sec();
    check("attach_ok", a > 0, a);
    shm_stat(id, &ds);
    check("nattch_1", ds.nattch == 1, (i64)ds.nattch);
    check("lpid_self", ds.lpid == getpid(), ds.lpid);
    check("atime_now", ds.atime >= t0 && ds.atime <= t1, ds.atime - t0);
    check("ctime_unchanged", ds.ctime == ctime, ds.ctime - ctime);
    check("dtime_still_0", ds.dtime == 0, ds.dtime);
    check("segsz_unchanged", ds.segsz == 100, (i64)ds.segsz);

    i64 b = shmat(id, 0, SHM_RDONLY);
    shm_stat(id, &ds);
    check("rdonly_counts_too", b > 0 && ds.nattch == 2, (i64)ds.nattch);
    shmdt(a);
    shmdt(b);
    shm_rm(id);
    return finish();
}
