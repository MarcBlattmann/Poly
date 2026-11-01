/* IPC_INFO fills a struct seminfo (ten ints) with the limits and returns
 * the highest index in use (>= 0); the semid is not looked up (any
 * non-negative value works). Defaults: semmap 1024000000, semmni 32000,
 * semmns 1024000000, semmnu 1024000000, semmsl 32000, semopm 500,
 * semume 500, semusz 20, semvmx 32767, semaem 32767. SEM_INFO is the
 * same but semusz = number of sets in use and semaem = number of
 * semaphores in use. An unwritable buffer is EFAULT. SEM_STAT and
 * SEM_STAT_ANY take an index (the set's id modulo 32768) and return the
 * set's id after filling the semid64_ds; an index not in use is EINVAL.
 * poly-diff: syscalls=names (semaphore ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_semget 64
#define SYS_semop 65
#define SYS_semctl 66
#define SYS_semtimedop 220
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define EFBIG 27
#define ENOSPC 28
#define ERANGE 34
#define EIDRM 43
/* linux/ipc.h, linux/sem.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define GETPID 11
#define GETVAL 12
#define GETALL 13
#define GETNCNT 14
#define GETZCNT 15
#define SETVAL 16
#define SETALL 17
#define SEM_STAT 18
#define SEM_INFO 19
#define SEM_STAT_ANY 20
#define SEM_UNDO 0x1000

/* struct sembuf (linux/sem.h) */
struct k_sembuf {
    unsigned short num;
    short op;
    short flg;
};

/* struct semid64_ds (arch/x86/include/uapi/asm/sembuf.h, struct
 * ipc64_perm from asm-generic/ipcbuf.h) on x86-64: 104 bytes. */
struct k_semid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    i64 otime;
    u64 otime_high;
    i64 ctime;
    u64 ctime_high;
    u64 nsems;
    u64 unused3, unused4;
};

static __attribute__((unused)) i64 semget(i64 key, i64 nsems, i64 flg) { return syscall3(SYS_semget, key, nsems, flg); }
static __attribute__((unused)) i64 semop(i64 id, const void *sops, u64 n) { return syscall3(SYS_semop, id, sops, n); }
static __attribute__((unused)) i64 semtimedop(i64 id, const void *sops, u64 n, const void *ts) {
    return syscall4(SYS_semtimedop, id, sops, n, ts);
}
static __attribute__((unused)) i64 semctl(i64 id, i64 num, i64 cmd, u64 arg) { return syscall4(SYS_semctl, id, num, cmd, arg); }
static __attribute__((unused)) i64 sem_rm(i64 id) { return semctl(id, 0, IPC_RMID, 0); }
static __attribute__((unused)) i64 sem_val(i64 id, int n) { return semctl(id, n, GETVAL, 0); }
static __attribute__((unused)) i64 sem_set(i64 id, int n, int v) { return semctl(id, n, SETVAL, (u64)v); }
static __attribute__((unused)) i64 sem_stat(i64 id, struct k_semid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return semctl(id, 0, IPC_STAT, (u64)ds);
}
static __attribute__((unused)) i64 sem_op1(i64 id, int num, int op, int flg) {
    struct k_sembuf b = { (unsigned short)num, (short)op, (short)flg };
    return semop(id, &b, 1);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }

struct k_seminfo {
    int semmap, semmni, semmns, semmnu, semmsl, semopm, semume, semusz, semvmx, semaem;
};

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 5, 0600);
    check("semget_ok", id >= 0, id);
    struct k_seminfo si;
    memset(&si, 0xff, sizeof si);
    i64 r = semctl(0x7ffffff0, 0, IPC_INFO, (u64)&si);
    check("ipc_info_ok", r >= (id & 0x7fff), r);
    check("semmap", si.semmap == 1024000000, si.semmap);
    check("semmni", si.semmni == 32000, si.semmni);
    check("semmns", si.semmns == 1024000000, si.semmns);
    check("semmnu", si.semmnu == 1024000000, si.semmnu);
    check("semmsl", si.semmsl == 32000, si.semmsl);
    check("semopm", si.semopm == 500, si.semopm);
    check("semume", si.semume == 500, si.semume);
    check("semusz", si.semusz == 20, si.semusz);
    check("semvmx", si.semvmx == 32767, si.semvmx);
    check("semaem", si.semaem == 32767, si.semaem);

    memset(&si, 0xff, sizeof si);
    i64 r2 = semctl(0, 0, SEM_INFO, (u64)&si);
    check("sem_info_same_return", r2 == r, r2);
    check("sem_info_semmsl", si.semmsl == 32000, si.semmsl);
    check("sem_info_sets_in_use", si.semusz >= 1, si.semusz);
    check("sem_info_sems_in_use", si.semaem >= 5, si.semaem);
    r = semctl(0, 0, IPC_INFO, 0);
    check("ipc_info_null_efault", r == -EFAULT, r);
    r = semctl(0, 0, SEM_INFO, 0);
    check("sem_info_null_efault", r == -EFAULT, r);

    struct k_semid_ds ds;
    memset(&ds, 0, sizeof ds);
    r = semctl(id & 0x7fff, 0, SEM_STAT, (u64)&ds);
    check("sem_stat_returns_id", r == id, r);
    check("sem_stat_nsems", ds.nsems == 5, (i64)ds.nsems);
    check("sem_stat_mode", ds.mode == 0600, ds.mode);
    memset(&ds, 0, sizeof ds);
    r = semctl(id & 0x7fff, 0, SEM_STAT_ANY, (u64)&ds);
    check("sem_stat_any_returns_id", r == id, r);
    check("sem_stat_any_nsems", ds.nsems == 5, (i64)ds.nsems);
    r = semctl(32767, 0, SEM_STAT, (u64)&ds);
    check("sem_stat_unused_index_einval", r == -EINVAL, r);
    r = semctl(id & 0x7fff, 0, SEM_STAT, 0);
    check("sem_stat_null_efault", r == -EFAULT, r);
    sem_rm(id);
    r = semctl(id & 0x7fff, 0, SEM_STAT, (u64)&ds);
    check("sem_stat_removed_einval", r == -EINVAL, r);
    return finish();
}
