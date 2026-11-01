/* A negative shmid or negative command is EINVAL for every command
 * (also IPC_INFO and SHM_INFO); an unknown command is EINVAL (4-10, 16
 * and up, IPC_STAT with the IPC_64 bit 0x100); an id not in use or
 * removed is EINVAL (also for SHM_LOCK); IPC_SET reads its buffer before
 * the id is looked up. Only the low 32 bits of shmid and cmd count.
 * poly-diff: syscalls=names (segment ids depend on the IPC namespace's history)
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
    struct k_shmid_ds ds;
    char info[72];
    i64 r = shmctl(-1, IPC_STAT, &ds);
    check("negative_stat_einval", r == -EINVAL, r);
    r = shmctl(-1, IPC_INFO, info);
    check("negative_ipc_info_einval", r == -EINVAL, r);
    r = shmctl(-1, SHM_INFO, info);
    check("negative_shm_info_einval", r == -EINVAL, r);
    r = shmctl(0, -1, info);
    check("negative_cmd_einval", r == -EINVAL, r);
    r = shmctl(0x7ffffff0, IPC_STAT, &ds);
    check("unused_stat_einval", r == -EINVAL, r);
    r = shmctl(0x7ffffff0, IPC_RMID, 0);
    check("unused_rmid_einval", r == -EINVAL, r);
    r = shmctl(0x7ffffff0, SHM_LOCK, 0);
    check("unused_lock_einval", r == -EINVAL, r);
    r = shmctl(0x7ffffff0, IPC_SET, 0);
    check("set_efault_before_id", r == -EFAULT, r);

    i64 id = shmget(IPC_PRIVATE, 100, 0600);
    check("shmget_ok", id >= 0, id);
    r = shmctl(id, 4, &ds);
    check("cmd_4_einval", r == -EINVAL, r);
    r = shmctl(id, 10, &ds);
    check("cmd_10_einval", r == -EINVAL, r);
    r = shmctl(id, 16, &ds);
    check("cmd_16_einval", r == -EINVAL, r);
    r = shmctl(id, IPC_STAT | 0x100, &ds);
    check("ipc64_bit_einval", r == -EINVAL, r);
    r = shmctl(id | (1L << 32), (1L << 32) | IPC_STAT, &ds);
    check("high_bits_ignored", r == 0 && ds.segsz == 100, r);
    shm_rm(id);
    r = shmctl(id, IPC_STAT, &ds);
    check("removed_stat_einval", r == -EINVAL, r);
    r = shmctl(id, IPC_RMID, 0);
    check("removed_rmid_einval", r == -EINVAL, r);
    r = shmctl(id, SHM_UNLOCK, 0);
    check("removed_unlock_einval", r == -EINVAL, r);
    memset(&ds, 0, sizeof ds);
    r = shmctl(id, IPC_SET, &ds);
    check("removed_set_einval", r == -EINVAL, r);
    return finish();
}
