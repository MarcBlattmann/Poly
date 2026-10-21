/* Attaching checks the segment's mode for the caller's class (owner
 * here): read-write needs r and w, SHM_RDONLY needs r, SHM_EXEC adds x.
 * Missing: EACCES. Only the owner bits count for the owner, even when
 * group or other bits would allow more. A caller with CAP_IPC_OWNER
 * (root) passes every check.
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
static int root;
/* Attach `id` with `flg` and detach again: 0 when it worked, else the error. */
static i64 try_at(i64 id, i64 flg) {
    i64 r = shmat(id, 0, flg);
    if (r < 0 && r > -4096) return r;
    shmdt(r);
    return 0;
}
static i64 denied(void) { return root ? 0 : -EACCES; }
int test_main(void) {
    root = is_root();
    i64 id = shmget(IPC_PRIVATE, 100, 0600);
    check("rw_seg_rw", try_at(id, 0) == 0, try_at(id, 0));
    check("rw_seg_ro", try_at(id, SHM_RDONLY) == 0, try_at(id, SHM_RDONLY));
    check("rw_seg_exec_denied", try_at(id, SHM_EXEC) == denied(), try_at(id, SHM_EXEC));
    check("rw_seg_exec_ro_denied", try_at(id, SHM_EXEC | SHM_RDONLY) == denied(), try_at(id, SHM_EXEC | SHM_RDONLY));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0400);
    check("r_seg_rw_denied", try_at(id, 0) == denied(), try_at(id, 0));
    check("r_seg_ro", try_at(id, SHM_RDONLY) == 0, try_at(id, SHM_RDONLY));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0200);
    check("w_seg_rw_denied", try_at(id, 0) == denied(), try_at(id, 0));
    check("w_seg_ro_denied", try_at(id, SHM_RDONLY) == denied(), try_at(id, SHM_RDONLY));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0);
    check("mode0_rw_denied", try_at(id, 0) == denied(), try_at(id, 0));
    check("mode0_ro_denied", try_at(id, SHM_RDONLY) == denied(), try_at(id, SHM_RDONLY));
    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("denied_no_nattch", ds.nattch == 0, (i64)ds.nattch);
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0077);
    check("owner_bits_rule_rw", try_at(id, 0) == denied(), try_at(id, 0));
    check("owner_bits_rule_ro", try_at(id, SHM_RDONLY) == denied(), try_at(id, SHM_RDONLY));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0700);
    check("rwx_seg_exec", try_at(id, SHM_EXEC) == 0, try_at(id, SHM_EXEC));
    check("rwx_seg_exec_ro", try_at(id, SHM_EXEC | SHM_RDONLY) == 0, try_at(id, SHM_EXEC | SHM_RDONLY));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0500);
    check("rx_seg_exec_ro", try_at(id, SHM_EXEC | SHM_RDONLY) == 0, try_at(id, SHM_EXEC | SHM_RDONLY));
    check("rx_seg_exec_rw_denied", try_at(id, SHM_EXEC) == denied(), try_at(id, SHM_EXEC));
    shm_rm(id);

    id = shmget(IPC_PRIVATE, 100, 0300);
    check("wx_seg_exec_rw_denied", try_at(id, SHM_EXEC) == denied(), try_at(id, SHM_EXEC));
    shm_rm(id);
    return finish();
}
