/* shmflg bits that are not permissions, IPC_CREAT or IPC_EXCL: only the
 * low 9 bits become the mode; SHM_NORESERVE (010000) and unknown high
 * bits (0x100000, 0x80000000) are accepted and leave no trace in the
 * mode. IPC_CREAT and IPC_EXCL are not stored either.
 * poly-diff: syscalls=names (segment ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
/* linux/ipc.h, linux/shm.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_STAT 2
#define SHM_RDONLY 010000
#define SHM_NORESERVE 010000
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
static __attribute__((unused)) i64 shm_rm(i64 id) { return syscall3(SYS_shmctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_shmctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 shm_attach(i64 id, i64 flg) { return syscall3(SYS_shmat, id, 0, flg); }
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x50000000 | (tag << 22) | (int)getpid()); }
int test_main(void) {
    struct k_shmid_ds ds;
    i64 id = shmget(IPC_PRIVATE, 100, 0754 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    shm_stat(id, &ds);
    check("mode_low_bits_only", ds.mode == 0754, ds.mode);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0600 | SHM_NORESERVE);
    check("noreserve_ok", id >= 0, id);
    shm_stat(id, &ds);
    check("noreserve_not_in_mode", ds.mode == 0600, ds.mode);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0600 | 0x100000);
    check("bit20_ok", id >= 0, id);
    shm_stat(id, &ds);
    check("bit20_not_in_mode", ds.mode == 0600, ds.mode);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0600 | 0x80000000L);
    check("bit31_ok", id >= 0, id);
    shm_stat(id, &ds);
    check("bit31_not_in_mode", ds.mode == 0600, ds.mode);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0600 | 0xffff00000000L);
    check("upper_half_ignored", id >= 0, id);
    shm_stat(id, &ds);
    check("upper_half_not_in_mode", ds.mode == 0600, ds.mode);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0777);
    shm_stat(id, &ds);
    check("mode_0777", ds.mode == 0777, ds.mode);
    shm_rm(id);
    return finish();
}
