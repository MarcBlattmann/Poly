/* shmctl for a caller that is not the owner (a child dropped to uid/gid
 * 65534, no capabilities): IPC_STAT and SHM_STAT need r in the
 * segment's "other" bits (EACCES), SHM_STAT_ANY needs nothing; IPC_SET,
 * IPC_RMID, SHM_LOCK and SHM_UNLOCK need euid == uid or cuid (EPERM,
 * whatever the mode). After root gives the segment to 65534 it is the
 * owner and may remove it.
 * poly-diff: needs=root (setresuid to another user)
 * poly-diff: syscalls=off (several processes)
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

static int drop(void) {
    if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) return 0;
    return syscall3(SYS_setresuid, 65534, 65534, 65534) == 0;
}

int test_main(void) {
    i64 a = shmget(IPC_PRIVATE, 4096, 0600);
    i64 b = shmget(IPC_PRIVATE, 4096, 0666);
    check("shmget_ok", a >= 0 && b >= 0, a);
    i64 pid = fork();
    if (pid == 0) {
        if (!drop()) exit_group(100);
        struct k_shmid_ds ds;
        int bad = 0;
        if (shm_stat(a, &ds) != -EACCES) bad |= 1;
        if (shmctl(a & 0x7fff, SHM_STAT, &ds) != -EACCES) bad |= 2;
        if (shmctl(a & 0x7fff, SHM_STAT_ANY, &ds) != a) bad |= 4;
        if (shm_rm(a) != -EPERM) bad |= 8;
        if (shm_stat(b, &ds) != 0) bad |= 16;
        if (shmctl(b, IPC_SET, &ds) != -EPERM) bad |= 32;
        if (shm_rm(b) != -EPERM) bad |= 64;
        if (shmctl(b, SHM_LOCK, 0) != -EPERM) bad |= 128;
        if (shmctl(b, SHM_UNLOCK, 0) != -EPERM) bad |= 256;
        exit_group(bad);
    }
    i64 st = wait_exit(pid);
    check("other_user_checks", st == 0, st);

    struct k_shmid_ds ds;
    shm_stat(a, &ds);
    ds.uid = 65534;
    i64 r = shmctl(a, IPC_SET, &ds);
    check("give_away_ok", r == 0, r);
    pid = fork();
    if (pid == 0) {
        if (!drop()) exit_group(100);
        struct k_shmid_ds d2;
        if (shm_stat(a, &d2) != 0 || d2.cuid != 0) exit_group(1);
        exit_group(shm_rm(a) == 0 ? 0 : 2);
    }
    st = wait_exit(pid);
    check("new_owner_removes", st == 0, st);
    r = shm_stat(a, &ds);
    check("removed", r == -EINVAL, r);
    shm_rm(b);
    return finish();
}
