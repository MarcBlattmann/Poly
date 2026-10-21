/* shmat(id, NULL, flags) attaches the whole segment at a page-aligned
 * address the system picks and returns that address. Each call makes a
 * new attachment at a different address; all attachments of one id
 * share the same bytes, and a SHM_RDONLY attachment sees writes made
 * through another. The id and the flags are 32-bit ints: bits above 31
 * are ignored; unknown flag bits are ignored too.
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
    i64 id = shmget(IPC_PRIVATE, 8192, 0600);
    check("create_ok", id >= 0, id);
    i64 a = shmat(id, 0, 0);
    check("attach_ok", a > 0, a);
    check("page_aligned", (a & 4095) == 0, a & 4095);
    i64 b = shmat(id, 0, 0);
    check("second_attach_ok", b > 0, b);
    check("second_attach_elsewhere", b != a && (b + 8192 <= a || a + 8192 <= b), b - a);
    i64 r = shmat(id, 0, SHM_RDONLY);
    check("rdonly_attach_ok", r > 0 && (r & 4095) == 0, r);
    volatile unsigned char *pa = (volatile unsigned char *)a, *pb = (volatile unsigned char *)b,
                           *pr = (volatile unsigned char *)r;
    pa[0] = 0x11;
    pa[8191] = 0x22;
    pb[100] = 0x33;
    check("shared_a_to_b", pb[0] == 0x11 && pb[8191] == 0x22, pb[0]);
    check("shared_b_to_a", pa[100] == 0x33, pa[100]);
    check("rdonly_sees_writes", pr[0] == 0x11 && pr[100] == 0x33 && pr[8191] == 0x22, pr[0]);

    i64 h = shmat(id | (1L << 32), 0, 0);
    check("id_high_bits_ignored", h > 0, h);
    i64 f = shmat(id, 0, 0x1000000 | 0x40000000);
    check("unknown_flags_ignored", f > 0, f);
    i64 g = shmat(id, 0, (1L << 32) | SHM_RDONLY);
    check("flag_high_bits_ignored", g > 0, g);
    volatile unsigned char *pg = (volatile unsigned char *)g, *ph = (volatile unsigned char *)h;
    check("all_share", pg[0] == 0x11 && ph[8191] == 0x22, pg[0]);

    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("nattch_6", ds.nattch == 6, (i64)ds.nattch);
    shmdt(a); shmdt(b); shmdt(r); shmdt(h); shmdt(f); shmdt(g);
    shm_stat(id, &ds);
    check("nattch_0_after_detach", ds.nattch == 0, (i64)ds.nattch);
    shm_rm(id);
    return finish();
}
