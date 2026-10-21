/* A segment removed (shmctl IPC_RMID) while still attached stays usable
 * by its id, but its key is released at once: shmget(key) is ENOENT,
 * IPC_STAT reports key 0 and SHM_DEST (01000) in the mode, and
 * IPC_CREAT|IPC_EXCL with the key makes a new segment with a different
 * id. Once the last attachment goes, the old id is EINVAL.
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
    i64 id = shmget(key, 4096, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 p = shm_attach(id, 0);
    check("attach_ok", p > 0, p);
    ((volatile char *)p)[0] = 9;

    i64 r = shm_rm(id);
    check("rm_attached_ok", r == 0, r);
    r = shmget(key, 0, 0);
    check("key_released_enoent", r == -ENOENT, r);
    struct k_shmid_ds ds;
    r = shm_stat(id, &ds);
    check("old_id_still_stat", r == 0, r);
    check("old_key_zero", ds.key == 0, ds.key);
    check("mode_has_dest", ds.mode == (SHM_DEST | 0600), ds.mode);
    check("still_attached", ds.nattch == 1, (i64)ds.nattch);

    i64 id2 = shmget(key, 100, 0600 | IPC_CREAT | IPC_EXCL);
    check("recreate_ok", id2 >= 0, id2);
    check("recreate_new_id", id2 != id, id2);
    r = shmget(key, 0, 0);
    check("key_finds_new", r == id2, r);
    check("old_memory_kept", ((volatile char *)p)[0] == 9, 0);

    syscall1(SYS_shmdt, p);
    r = shm_stat(id, &ds);
    check("old_id_gone_after_detach", r == -EINVAL, r);
    r = shmget(key, 0, 0);
    check("new_still_found", r == id2, r);
    shm_rm(id2);
    r = shmget(key, 0, 0);
    check("enoent_after_rm", r == -ENOENT, r);
    return finish();
}
