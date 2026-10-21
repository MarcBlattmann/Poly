/* Ids: 64 live segments all have distinct non-negative ids; an id that
 * was just removed is not handed out again by the next shmget; a removed
 * unattached segment is gone at once (IPC_STAT EINVAL, its key ENOENT,
 * IPC_CREAT makes a new one).
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
    i64 ids[64];
    int ok = 1, distinct = 1;
    for (int i = 0; i < 64; i++) {
        ids[i] = shmget(IPC_PRIVATE, 16, 0600);
        if (ids[i] < 0) ok = 0;
        for (int j = 0; j < i; j++)
            if (ids[j] == ids[i]) distinct = 0;
    }
    check("64_created", ok, 0);
    check("64_distinct", distinct, 0);
    int rm_ok = 1;
    for (int i = 0; i < 64; i++)
        if (shm_rm(ids[i]) != 0) rm_ok = 0;
    check("64_removed", rm_ok, 0);

    i64 a = shmget(IPC_PRIVATE, 16, 0600);
    shm_rm(a);
    i64 b = shmget(IPC_PRIVATE, 16, 0600);
    check("removed_id_not_reused_next", b >= 0 && b != a, b);
    shm_rm(b);

    int key = own_key(2);
    i64 id = shmget(key, 16, 0600 | IPC_CREAT);
    i64 r = shm_rm(id);
    check("rm_ok", r == 0, r);
    struct k_shmid_ds ds;
    r = shm_stat(id, &ds);
    check("removed_stat_einval", r == -EINVAL, r);
    r = shmget(key, 16, 0600);
    check("removed_key_enoent", r == -ENOENT, r);
    i64 id2 = shmget(key, 16, 0600 | IPC_CREAT);
    check("recreated_new_id", id2 >= 0 && id2 != id, id2);
    shm_rm(id2);
    return finish();
}
