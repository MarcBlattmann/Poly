/* shmdt fails with EINVAL, changing nothing, for every address that is not
 * the start of an attachment: 0, an unaligned address inside the
 * attachment, a later page of it, the start of an ordinary anonymous
 * mapping, the start of a shared file-less mmap, free address space, and
 * addresses at or above the end of user space (kernel half,
 * non-canonical, all ones). The address is a full 64-bit value: the
 * attach address with bit 32 flipped or bit 63 set is not the attachment.
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
    i64 id = shmget(IPC_PRIVATE, 3 * 4096, 0600);
    i64 a = shmat(id, 0, 0);
    check("attach_ok", a > 0, a);
    i64 anon = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    i64 shan = syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    check("mmaps_ok", anon > 0 && shan > 0, anon);
    check("zero", shmdt(0) == -EINVAL, shmdt(0));
    check("unaligned_1", shmdt(a + 1) == -EINVAL, shmdt(a + 1));
    check("unaligned_2048", shmdt(a + 2048) == -EINVAL, shmdt(a + 2048));
    check("second_page", shmdt(a + 4096) == -EINVAL, shmdt(a + 4096));
    check("last_page", shmdt(a + 2 * 4096) == -EINVAL, shmdt(a + 2 * 4096));
    check("page_before", shmdt(a - 4096) == -EINVAL, shmdt(a - 4096));
    check("anon_private", shmdt(anon) == -EINVAL, shmdt(anon));
    check("anon_shared", shmdt(shan) == -EINVAL, shmdt(shan));
    u64 hole = free_range(4096);
    check("free_space", shmdt(hole) == -EINVAL, shmdt(hole));
    check("end_of_user", shmdt(0x7ffffffff000L) == -EINVAL, shmdt(0x7ffffffff000L));
    check("non_canonical", shmdt(0x0000800000000000L) == -EINVAL, shmdt(0x0000800000000000L));
    check("kernel_half", (shmdt((i64)0xffff800000000000UL) == -EINVAL), 0);
    check("all_ones", shmdt(-1) == -EINVAL, shmdt(-1));
    check("bit32_flipped", shmdt(a ^ (1L << 32)) == -EINVAL, 0);
    check("bit63_added", shmdt((i64)((u64)a | (1UL << 63))) == -EINVAL, 0);
    check("nattch_still_1", nattch(id) == 1, nattch(id));
    check("still_mapped", mapped(a) && mapped(a + 2 * 4096) && mapped(anon) && mapped(shan), 0);
    i64 r = shmdt(a);
    check("real_shmdt_ok", r == 0, r);
    syscall2(SYS_munmap, anon, 4096);
    syscall2(SYS_munmap, shan, 4096);
    shm_rm(id);
    return finish();
}
