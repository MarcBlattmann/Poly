/* An attachment covers the segment's size rounded up to whole pages: a
 * 1-byte segment gives one full page (zeroed, all of it usable); the
 * byte after that page is not mapped (SIGSEGV). The overlap check uses
 * the rounded size too: a 4097-byte segment needs two free pages, a
 * 4096-byte one only one.
 * poly-diff: syscalls=off (two processes)
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
    u64 hole = free_range(4 * 4096) + 4096;
    i64 one = shmget(IPC_PRIVATE, 1, 0600);
    i64 p = shmat(one, hole, 0);
    check("attach_ok", p == (i64)hole, p - (i64)hole);
    volatile unsigned char *b = (volatile unsigned char *)p;
    int zero = 1;
    for (int i = 0; i < 4096; i++)
        if (b[i]) zero = 0;
    check("whole_page_zero", zero, 0);
    /* The page before `hole` and the two after its page are free. */
    i64 two = shmget(IPC_PRIVATE, 4097, 0600);
    i64 r = shmat(two, hole - 4096, 0);
    check("4097_needs_two_pages", r == -EINVAL, r);
    i64 page = shmget(IPC_PRIVATE, 4096, 0600);
    r = shmat(page, hole - 4096, 0);
    check("4096_fits_one_page", r == (i64)(hole - 4096), r);
    shmdt(r);
    r = shmat(two, hole + 4096, 0);
    check("4097_fits_two_pages", r == (i64)(hole + 4096), r);
    shmdt(r);

    i64 pid = fork();
    if (pid == 0) {
        volatile unsigned char c = b[4096];
        (void)c;
        exit_group(0);
    }
    int s = wait_status(pid);
    check("past_page_segv", W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV, s);
    b[4095] = 0x5a;
    i64 q = shmat(one, 0, SHM_RDONLY);
    check("whole_page_shared", q > 0 && ((volatile unsigned char *)q)[4095] == 0x5a, 0);

    shmdt(p);
    shmdt(q);
    shm_rm(one);
    shm_rm(two);
    shm_rm(page);
    return finish();
}
