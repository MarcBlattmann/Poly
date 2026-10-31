/* shmdt of an address that is not the start of a shared memory
 * attachment is EINVAL and changes nothing: an unaligned address (also
 * one inside the attachment), NULL, the second page of an attachment,
 * an ordinary anonymous mapping (it stays mapped), an unmapped hole, a
 * kernel or non-canonical address.
 * poly-diff: syscalls=names (attach addresses vary)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
#define SYS_setrlimit 160
#define SYS_getrlimit 97
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EIDRM 43
/* linux/ipc.h, linux/shm.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define SHM_LOCK 11
#define SHM_UNLOCK 12
#define SHM_STAT 13
#define SHM_INFO 14
#define SHM_STAT_ANY 15
#define SHM_RDONLY 010000
#define SHM_DEST 01000
#define SHM_LOCKED 02000
/* asm-generic/resource.h */
#define RLIMIT_MEMLOCK 8

/* struct shmid64_ds (asm-generic/shmbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64: 112 bytes. */
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
static __attribute__((unused)) i64 shmctl(i64 id, i64 cmd, void *buf) { return syscall3(SYS_shmctl, id, cmd, buf); }
static __attribute__((unused)) i64 shm_rm(i64 id) { return shmctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return shmctl(id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 shm_attach(i64 id, i64 flg) { return syscall3(SYS_shmat, id, 0, flg); }
static __attribute__((unused)) i64 shmdt(u64 addr) { return syscall1(SYS_shmdt, addr); }
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }
/* Whether touching `p` (reading one byte) kills a child with SIGSEGV. */
static __attribute__((unused)) int read_faults(const volatile char *p) {
    i64 pid = fork();
    if (pid == 0) {
        (void)*p;
        exit_group(0);
    }
    int s = wait_status(pid);
    return s != -1 && W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV;
}

int test_main(void) {
    i64 id = shmget(IPC_PRIVATE, 2 * 4096, 0600);
    check("shmget_ok", id >= 0, id);
    char *p = (char *)shm_attach(id, 0);
    check("attach_ok", (i64)p > 0, (i64)p);
    p[0] = 1;
    i64 r = shmdt((u64)p + 1);
    check("unaligned_einval", r == -EINVAL, r);
    r = shmdt((u64)p + 4095);
    check("unaligned_end_einval", r == -EINVAL, r);
    r = shmdt(0);
    check("null_einval", r == -EINVAL, r);
    r = shmdt((u64)p + 4096);
    check("second_page_einval", r == -EINVAL, r);
    r = shmdt((u64)p - 4096);
    check("page_before_einval", r == -EINVAL, r);

    char *a = (char *)syscall6(SYS_mmap, 0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    a[0] = 7;
    r = shmdt((u64)a);
    check("anonymous_mapping_einval", r == -EINVAL, r);
    check("anonymous_still_mapped", a[0] == 7, a[0]);
    char *hole = (char *)syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, hole, 4096);
    r = shmdt((u64)hole);
    check("hole_einval", r == -EINVAL, r);
    r = shmdt(0xffff800000000000UL);
    check("kernel_einval", r == -EINVAL, r);
    r = shmdt(0x0000800000000000UL);
    check("non_canonical_einval", r == -EINVAL, r);
    r = shmdt((u64)p | (1UL << 63));
    check("top_bit_einval", r == -EINVAL, r);

    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("still_attached", ds.nattch == 1 && ds.dtime == 0, (i64)ds.nattch);
    check("still_usable", p[0] == 1, p[0]);
    r = shmdt((u64)p);
    check("real_detach_ok", r == 0, r);
    shm_rm(id);
    return finish();
}
