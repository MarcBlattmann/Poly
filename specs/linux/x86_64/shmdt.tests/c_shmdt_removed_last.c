/* A segment marked for removal (IPC_RMID) while attached stays until the
 * last attachment goes: after one of two shmdt calls IPC_STAT still works
 * (nattch 1, SHM_DEST set, key 0) and the remaining attachment keeps its
 * bytes; the last shmdt destroys it: IPC_STAT, shmat and IPC_RMID by that
 * id are EINVAL from then on.
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
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    i64 a = shmat(id, 0, 0);
    i64 b = shmat(id, 0, 0);
    check("attach_ok", a > 0 && b > 0, b);
    ((volatile char *)a)[1] = 9;
    check("rmid_ok", shm_rm(id) == 0, 0);
    struct k_shmid_ds ds;
    i64 r = shm_stat(id, &ds);
    check("stat_after_rmid", r == 0 && ds.nattch == 2 && (ds.mode & SHM_DEST), ds.mode);
    r = shmdt(a);
    check("first_shmdt_ok", r == 0, r);
    r = shm_stat(id, &ds);
    check("stat_still_ok", r == 0, r);
    check("nattch_1", ds.nattch == 1, (i64)ds.nattch);
    check("dest_set", (ds.mode & SHM_DEST) != 0, ds.mode);
    check("key_0", ds.key == 0, ds.key);
    check("bytes_kept", ((volatile char *)b)[1] == 9, ((volatile char *)b)[1]);
    r = shmdt(b);
    check("last_shmdt_ok", r == 0, r);
    r = shm_stat(id, &ds);
    check("stat_einval", r == -EINVAL, r);
    r = shmat(id, 0, 0);
    check("shmat_einval", r == -EINVAL, r);
    r = shm_rm(id);
    check("rmid_einval", r == -EINVAL, r);
    check("unmapped", !mapped(a) && !mapped(b), 0);
    return finish();
}
