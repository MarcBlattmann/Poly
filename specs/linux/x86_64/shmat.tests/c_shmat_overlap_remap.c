/* Attaching at an address whose range (the segment rounded up to whole
 * pages) meets an existing mapping is EINVAL, whether the overlap is at
 * the start or only at a later page. With SHM_REMAP the segment
 * replaces whatever was mapped in its range; pages after the range keep
 * their contents. SHM_REMAP with a NULL address is EINVAL, also when
 * SHM_RND rounds the address down to 0.
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
    i64 id = shmget(IPC_PRIVATE, 4097, 0600);
    check("create_ok", id >= 0, id);
    i64 a = shmat(id, 0, 0);
    ((volatile char *)a)[4096] = 9;

    i64 r = shmat(id, (u64)a, 0);
    check("same_range_einval", r == -EINVAL, r);
    r = shmat(id, (u64)a + 4096, 0);
    check("partial_overlap_einval", r == -EINVAL, r);

    i64 m = syscall6(SYS_mmap, 0, 4 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    volatile char *mp = (volatile char *)m;
    for (int i = 0; i < 4 * 4096; i++) mp[i] = 5;
    r = shmat(id, (u64)m, 0);
    check("over_anon_einval", r == -EINVAL, r);
    /* The second page of the segment's range would meet the mapping. */
    r = shmat(id, (u64)m - 4096, SHM_RDONLY);
    check("tail_overlap_einval", r == -EINVAL, r);
    check("anon_untouched", mp[0] == 5 && mp[4096] == 5, mp[0]);

    r = shmat(id, (u64)m, SHM_REMAP);
    check("remap_ok", r == m, r - m);
    check("remap_shows_segment", mp[0] == 0 && mp[4096] == 9 && mp[8191] == 0, mp[0]);
    check("after_range_kept", mp[8192] == 5 && mp[4 * 4096 - 1] == 5, mp[8192]);
    mp[1] = 3;
    check("remap_shared", ((volatile char *)a)[1] == 3, ((volatile char *)a)[1]);

    r = shmat(id, (u64)m + 2 * 4096 + 5, SHM_REMAP | SHM_RND);
    check("remap_rnd_ok", r == m + 2 * 4096, r - m);
    check("remap_rnd_shows_segment", mp[8192 + 4096] == 9, mp[8192 + 4096]);

    r = shmat(id, 0, SHM_REMAP);
    check("remap_null_einval", r == -EINVAL, r);
    r = shmat(id, 100, SHM_REMAP | SHM_RND);
    check("remap_rnd_to_0_einval", r == -EINVAL, r);
    r = shmat(id, 4095, SHM_REMAP | SHM_RND | SHM_RDONLY);
    check("remap_rnd_4095_einval", r == -EINVAL, r);

    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("nattch_3", ds.nattch == 3, (i64)ds.nattch);
    shmdt(a);
    shmdt(m);
    shmdt(m + 2 * 4096);
    shm_rm(id);
    return finish();
}
