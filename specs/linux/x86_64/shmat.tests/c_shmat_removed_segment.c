/* A segment removed with IPC_RMID while attached can still be attached by
 * its id (nattch goes up; IPC_STAT shows key 0 and SHM_DEST in the mode)
 * and its memory stays shared. Once the last attachment goes the id is
 * gone: shmat and IPC_STAT are EINVAL.
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
int test_main(void) {
    i64 id = shmget(0x5a000000 | getpid(), 4096, 0600 | 01000 | 02000);
    check("create_ok", id >= 0, id);
    i64 a = shmat(id, 0, 0);
    ((volatile char *)a)[0] = 6;
    check("rmid_ok", shm_rm(id) == 0, id);

    i64 b = shmat(id, 0, 0);
    check("attach_after_rmid", b > 0, b);
    check("same_memory", b > 0 && ((volatile char *)b)[0] == 6, 0);
    i64 c = shmat(id, 0, SHM_RDONLY);
    check("rdonly_after_rmid", c > 0, c);
    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("nattch_3", ds.nattch == 3, (i64)ds.nattch);
    check("dest_flag", (ds.mode & SHM_DEST) != 0, ds.mode);
    check("perm_kept", (ds.mode & 0777) == 0600, ds.mode);
    check("key_0", ds.key == 0, ds.key);

    shmdt(a);
    shmdt(c);
    check("still_there", shm_stat(id, &ds) == 0 && ds.nattch == 1, (i64)ds.nattch);
    shmdt(b);
    i64 r = shm_stat(id, &ds);
    check("gone_stat", r == -EINVAL, r);
    r = shmat(id, 0, 0);
    check("gone_attach", r == -EINVAL, r);
    return finish();
}
