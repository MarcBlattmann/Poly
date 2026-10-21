/* shmdt of an attach address returns 0 and removes the whole attachment
 * (every page of it is unmapped afterwards); the segment and its bytes
 * live on: a new attachment reads what was written. nattch goes back
 * down. A segment whose size is not a page multiple detaches all of its
 * rounded-up pages.
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
    i64 id = shmget(IPC_PRIVATE, 5000, 0600);
    check("shmget_ok", id >= 0, id);
    i64 a = shmat(id, 0, 0);
    check("attach_ok", a > 0, a);
    check("nattch_1", nattch(id) == 1, nattch(id));
    ((volatile char *)a)[0] = 'x';
    ((volatile char *)a)[4999] = 'y';
    check("both_pages_mapped", mapped(a) && mapped(a + 4096), 0);
    i64 r = shmdt(a);
    check("shmdt_returns_0", r == 0, r);
    check("first_page_unmapped", !mapped(a), 0);
    check("second_page_unmapped", !mapped(a + 4096), 0);
    check("nattch_0", nattch(id) == 0, nattch(id));
    i64 b = shmat(id, 0, SHM_RDONLY);
    check("reattach_ok", b > 0, b);
    check("bytes_kept", ((volatile char *)b)[0] == 'x' && ((volatile char *)b)[4999] == 'y', 0);
    r = shmdt(b);
    check("shmdt_rdonly_ok", r == 0, r);
    check("nattch_0_again", nattch(id) == 0, nattch(id));
    check("rm_ok", shm_rm(id) == 0, 0);
    return finish();
}
