/* Size against an existing segment: any size up to the segment's size
 * (including 0) finds it; a larger one is EINVAL, with or without
 * IPC_CREAT. IPC_CREAT|IPC_EXCL reports EEXIST before a size problem,
 * and a size problem is reported before a permission problem.
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
    int key = own_key(5);
    i64 id = shmget(key, 5000, 0600 | IPC_CREAT);
    check("create_ok", id >= 0, id);
    i64 r = shmget(key, 5000, 0);
    check("same_size_ok", r == id, r);
    r = shmget(key, 1, 0);
    check("smaller_ok", r == id, r);
    r = shmget(key, 0, 0);
    check("zero_ok", r == id, r);
    r = shmget(key, 5001, 0);
    check("larger_einval", r == -EINVAL, r);
    r = shmget(key, 8192, 0600 | IPC_CREAT);
    check("larger_creat_einval", r == -EINVAL, r);
    r = shmget(key, 8192, 0600 | IPC_CREAT | IPC_EXCL);
    check("larger_excl_eexist", r == -EEXIST, r);

    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("size_unchanged", ds.segsz == 5000, (i64)ds.segsz);
    shm_rm(id);

    /* Read-only for the owner: asking for write access with a bad size. */
    key = own_key(6);
    id = shmget(key, 100, 0400 | IPC_CREAT);
    check("ro_create_ok", id >= 0, id);
    r = shmget(key, 200, 0600);
    check("size_before_permission", r == -EINVAL, r);
    r = shmget(key, 200, 0600 | IPC_CREAT | IPC_EXCL);
    check("eexist_before_permission", r == -EEXIST, r);
    shm_rm(id);
    return finish();
}
