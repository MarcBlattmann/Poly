/* nsems: negative or above SEMMSL (32000, the first field of
 * /proc/sys/kernel/sem) is EINVAL before the key is looked at (so also
 * for an unused key without IPC_CREAT, and for a used key). Creating
 * needs nsems >= 1 (0 is EINVAL, nothing is made). Finding a used key
 * needs nsems <= the set's nsems (0 is fine), else EINVAL; EEXIST is
 * reported before that, EACCES after it. 32000 semaphores in one set
 * work.
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

int test_main(void) {
    int key = own_key(1);
    i64 r = semget(key, 32001, 0600);
    check("unused_too_many_einval", r == -EINVAL, r);
    r = semget(key, -1, 0600);
    check("unused_negative_einval", r == -EINVAL, r);
    r = semget(key, 0, 0600 | IPC_CREAT);
    check("create_zero_einval", r == -EINVAL, r);
    r = semget(key, 1, 0600);
    check("nothing_made", r == -ENOENT, r);
    r = semget(IPC_PRIVATE, 0, 0600);
    check("private_zero_einval", r == -EINVAL, r);
    r = semget(IPC_PRIVATE, 32001, 0600);
    check("private_too_many_einval", r == -EINVAL, r);
    r = semget(IPC_PRIVATE, -5, 0600);
    check("private_negative_einval", r == -EINVAL, r);

    i64 id = semget(key, 4, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_4_ok", id >= 0, id);
    r = semget(key, 4, 0);
    check("lookup_4_ok", r == id, r);
    r = semget(key, 0, 0);
    check("lookup_0_ok", r == id, r);
    r = semget(key, 5, 0);
    check("lookup_5_einval", r == -EINVAL, r);
    r = semget(key, 5, IPC_CREAT | 0600);
    check("creat_5_einval", r == -EINVAL, r);
    r = semget(key, 5, IPC_CREAT | IPC_EXCL | 0600);
    check("eexist_before_nsems", r == -EEXIST, r);
    r = semget(key, 32001, IPC_CREAT | IPC_EXCL | 0600);
    check("too_many_before_eexist", r == -EINVAL, r);
    r = semget(key, -1, 0);
    check("used_negative_einval", r == -EINVAL, r);
    sem_rm(id);

    i64 big = semget(IPC_PRIVATE, 32000, 0600);
    check("semmsl_ok", big >= 0, big);
    struct k_semid_ds ds;
    sem_stat(big, &ds);
    check("semmsl_nsems", ds.nsems == 32000, (i64)ds.nsems);
    r = sem_set(big, 31999, 7);
    check("last_sem_setval", r == 0, r);
    check("last_sem_getval", sem_val(big, 31999) == 7, sem_val(big, 31999));
    r = sem_val(big, 32000);
    check("past_last_einval", r == -EINVAL, r);
    sem_rm(big);
    return finish();
}
