/* Which error wins when several apply: first the address form (unaligned
 * without SHM_RND, SHM_REMAP without an address: EINVAL), then the id
 * (EINVAL), then permission (EACCES), last the placement (ENOMEM past the
 * end of user space, EINVAL for an overlap). Root (CAP_IPC_OWNER) skips
 * the permission step.
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
    int root = is_root();
    i64 none = shmget(IPC_PRIVATE, 4096, 0);
    i64 ok = shmget(IPC_PRIVATE, 4096, 0600);
    check("create_ok", none >= 0 && ok >= 0, none);
    i64 gone = shmget(IPC_PRIVATE, 4096, 0600);
    shm_rm(gone);
    u64 hole = free_range(4096);

    i64 r = shmat(none, hole + 1, 0);
    check("unaligned_before_access", r == -EINVAL, r);
    r = shmat(none, 0, SHM_REMAP);
    check("remap_null_before_access", r == -EINVAL, r);
    r = shmat(gone, hole + 1, 0);
    check("unaligned_bad_id", r == -EINVAL, r);
    r = shmat(gone, 0xffff800000000000UL, 0);
    check("bad_id_before_range", r == -EINVAL, r);
    r = shmat(-5, 0x7ffffffff000UL, SHM_RDONLY);
    check("negative_id_before_range", r == -EINVAL, r);
    r = shmat(none, 0xffff800000000000UL, 0);
    check("access_before_range", r == (root ? -ENOMEM : -EACCES), r);

    i64 a = shmat(ok, 0, 0);
    r = shmat(none, (u64)a, SHM_RDONLY);
    check("access_before_overlap", r == (root ? -EINVAL : -EACCES), r);
    r = shmat(ok, (u64)a, 0);
    check("overlap", r == -EINVAL, r);
    r = shmat(ok, 0x7ffffffff000UL, 0);
    check("range", r == -ENOMEM, r);
    shmdt(a);
    shm_rm(none);
    shm_rm(ok);
    return finish();
}
