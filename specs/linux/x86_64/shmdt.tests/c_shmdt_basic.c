/* shmdt(addr) with the address shmat returned detaches that attachment
 * and returns 0: the pages are unmapped (touching them is SIGSEGV),
 * nattch goes down, shm_dtime becomes now and shm_lpid the caller;
 * atime, ctime and segsz are kept. The segment and its contents remain
 * (a new attachment sees them). A second shmdt of the same address is
 * EINVAL. The address is a full 64-bit value.
 * poly-diff: syscalls=off (a forked child probes the detached page)
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
    i64 t0 = now_sec();
    i64 id = shmget(IPC_PRIVATE, 6000, 0600);
    check("shmget_ok", id >= 0, id);
    char *p = (char *)shm_attach(id, 0);
    check("attach_ok", (i64)p > 0, (i64)p);
    p[0] = 'A';
    p[5999] = 'Z';
    struct k_shmid_ds before, after;
    shm_stat(id, &before);
    check("nattch_1", before.nattch == 1, (i64)before.nattch);
    check("dtime_0", before.dtime == 0, before.dtime);

    i64 r = shmdt((u64)p);
    check("shmdt_ok", r == 0, r);
    shm_stat(id, &after);
    check("nattch_0", after.nattch == 0, (i64)after.nattch);
    check("dtime_now", after.dtime >= t0 && after.dtime <= now_sec(), after.dtime - t0);
    check("lpid_me", after.lpid == getpid(), 0);
    check("atime_kept", after.atime == before.atime, after.atime - before.atime);
    check("ctime_kept", after.ctime == before.ctime, after.ctime - before.ctime);
    check("segsz_kept", after.segsz == 6000, (i64)after.segsz);
    check("first_page_unmapped", read_faults(p), 0);
    check("second_page_unmapped", read_faults(p + 4096), 0);
    r = shmdt((u64)p);
    check("second_shmdt_einval", r == -EINVAL, r);

    char *q = (char *)shm_attach(id, 0);
    check("reattach_ok", (i64)q > 0, (i64)q);
    check("contents_kept", q[0] == 'A' && q[5999] == 'Z', q[0]);
    r = syscall1(SYS_shmdt, (u64)q | 0);
    check("shmdt_q_ok", r == 0, r);
    shm_rm(id);
    return finish();
}
