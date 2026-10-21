/* shmdt finds an attachment by the address its offset 0 is (or was)
 * mapped at, not by the first page still mapped. After munmap of the
 * first page of a 3-page attachment, shmdt of the next page (the start of
 * what is left) is EINVAL, and shmdt of the original attach address
 * succeeds and removes the rest, even when an anonymous mapping now fills
 * the first page (that page is left alone). Likewise with the first two
 * pages gone: the address one page in is EINVAL, the original one works.
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
    check("attach_a_ok", a > 0, a);
    i64 r = syscall2(SYS_munmap, a, 4096);
    check("munmap_head_ok", r == 0, r);
    check("nattch_1", nattch(id) == 1, nattch(id));
    r = shmdt(a + 4096);
    check("rest_start_einval", r == -EINVAL, r);
    r = shmdt(a - 4096);
    check("page_before_einval", r == -EINVAL, r);
    check("rest_mapped", mapped(a + 4096) && mapped(a + 2 * 4096), 0);
    r = shmdt(a);
    check("old_start_ok", r == 0, r);
    check("rest_unmapped", !mapped(a + 4096) && !mapped(a + 2 * 4096), 0);
    check("nattch_0", nattch(id) == 0, nattch(id));

    i64 b = shmat(id, 0, 0);
    check("attach_b_ok", b > 0, b);
    syscall2(SYS_munmap, b, 4096);
    r = syscall6(SYS_mmap, b, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
    check("anon_in_head", r == b, r);
    r = shmdt(b);
    check("old_start_over_anon_ok", r == 0, r);
    check("anon_kept", mapped(b), 0);
    check("shm_rest_unmapped", !mapped(b + 4096) && !mapped(b + 2 * 4096), 0);
    check("b_nattch_0", nattch(id) == 0, nattch(id));
    syscall2(SYS_munmap, b, 4096);

    i64 c = shmat(id, 0, 0);
    check("attach_c_ok", c > 0, c);
    syscall2(SYS_munmap, c, 2 * 4096);
    r = shmdt(c + 4096);
    check("one_page_in_einval", r == -EINVAL, r);
    check("c_nattch_1", nattch(id) == 1, nattch(id));
    r = shmdt(c);
    check("c_old_start_ok", r == 0, r);
    check("c_unmapped", !mapped(c + 2 * 4096), 0);
    check("c_nattch_0", nattch(id) == 0, nattch(id));
    shm_rm(id);
    return finish();
}
