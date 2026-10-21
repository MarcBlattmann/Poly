/* shmdt removes only the pieces of the attachment itself: when the middle
 * page of a 3-page attachment was unmapped and an anonymous mapping put in
 * its place, shmdt of the start unmaps the first and last page but leaves
 * the anonymous page (and its contents) alone.
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
    i64 r = syscall2(SYS_munmap, a + 4096, 4096);
    check("munmap_middle_ok", r == 0, r);
    r = syscall6(SYS_mmap, a + 4096, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    check("anon_in_hole", r == a + 4096, r);
    ((volatile char *)a)[4096 + 5] = 42;
    r = shmdt(a);
    check("shmdt_ok", r == 0, r);
    check("first_unmapped", !mapped(a), 0);
    check("last_unmapped", !mapped(a + 2 * 4096), 0);
    check("anon_kept", mapped(a + 4096), 0);
    check("anon_bytes_kept", ((volatile char *)a)[4096 + 5] == 42, ((volatile char *)a)[4096 + 5]);
    check("nattch_0", nattch(id) == 0, nattch(id));
    syscall2(SYS_munmap, a + 4096, 4096);
    shm_rm(id);
    return finish();
}
