/* Sizes when a segment is made: 0 is EINVAL (for IPC_PRIVATE and for
 * IPC_CREAT of a new key, which is then not created), and so is a size
 * above the system's maximum such as SIZE_MAX. 1 byte is fine.
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
    i64 r = shmget(IPC_PRIVATE, 0, 0600);
    check("private_size0_einval", r == -EINVAL, r);
    r = shmget(IPC_PRIVATE, 0, 0600 | IPC_CREAT | IPC_EXCL);
    check("private_excl_size0_einval", r == -EINVAL, r);
    r = shmget(IPC_PRIVATE, ~0UL, 0600);
    check("private_size_max_einval", r == -EINVAL, r);
    r = shmget(IPC_PRIVATE, 1UL << 63, 0600);
    check("private_size_2p63_einval", r == -EINVAL, r);

    int key = own_key(2);
    r = shmget(key, 0, 0600 | IPC_CREAT);
    check("creat_size0_einval", r == -EINVAL, r);
    r = shmget(key, ~0UL, 0600 | IPC_CREAT);
    check("creat_size_max_einval", r == -EINVAL, r);
    r = shmget(key, 100, 0);
    check("failed_creat_made_nothing", r == -ENOENT, r);

    i64 id = shmget(IPC_PRIVATE, 1, 0600);
    check("size1_ok", id >= 0, id);
    shm_rm(id);
    id = shmget(IPC_PRIVATE, 4096, 0600);
    check("size_page_ok", id >= 0, id);
    shm_rm(id);
    id = shmget(IPC_PRIVATE, 1 << 20, 0600);
    check("size_1mib_ok", id >= 0, id);
    shm_rm(id);
    return finish();
}
