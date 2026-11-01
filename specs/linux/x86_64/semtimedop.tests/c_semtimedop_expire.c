/* A relative timeout bounds the sleep: when it runs out the call fails
 * with EAGAIN (not ETIMEDOUT), nothing of the array is applied and the
 * caller no longer counts in GETNCNT/GETZCNT. A zero timeout fails at
 * once when the array would block, and succeeds at once when it does
 * not. Applies to decrements and to wait-for-zero. The timeout struct is
 * only read (it is not updated).
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
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);
    sem_set(id, 1, 1);

    struct k_timespec ts = { 0, 100000000 };
    struct k_sembuf b[2] = { { 1, -1, 0 }, { 0, -1, 0 } };
    i64 t0 = now_ns();
    i64 r = semtimedop(id, b, 2, &ts);
    i64 ms = (now_ns() - t0) / 1000000;
    check("expired_eagain", r == -EAGAIN, r);
    check("waited_at_least_100ms", ms >= 95, ms);
    check("waited_less_than_3s", ms < 3000, ms);
    check("first_op_not_applied", sem_val(id, 1) == 1, sem_val(id, 1));
    check("ncnt_0_after", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    check("timeout_not_updated", ts.tv_sec == 0 && ts.tv_nsec == 100000000, ts.tv_nsec);

    struct k_sembuf z = { 1, 0, 0 };
    struct k_timespec ts2 = { 0, 50000000 };
    t0 = now_ns();
    r = semtimedop(id, &z, 1, &ts2);
    ms = (now_ns() - t0) / 1000000;
    check("zero_wait_expired_eagain", r == -EAGAIN, r);
    check("zero_wait_at_least_50ms", ms >= 45 && ms < 3000, ms);
    check("zcnt_0_after", semctl(id, 1, GETZCNT, 0) == 0, semctl(id, 1, GETZCNT, 0));

    struct k_timespec zero = { 0, 0 };
    t0 = now_ns();
    r = semtimedop(id, b, 2, &zero);
    ms = (now_ns() - t0) / 1000000;
    check("zero_timeout_eagain", r == -EAGAIN, r);
    check("zero_timeout_quick", ms < 50, ms);
    struct k_sembuf ok = { 1, -1, 0 };
    r = semtimedop(id, &ok, 1, &zero);
    check("zero_timeout_possible_ok", r == 0, r);
    check("applied", sem_val(id, 1) == 0, sem_val(id, 1));

    /* A huge timeout is valid; with IPC_NOWAIT the call does not sleep. */
    struct k_timespec huge = { 1L << 40, 999999999 };
    struct k_sembuf nw = { 0, -1, IPC_NOWAIT };
    r = semtimedop(id, &nw, 1, &huge);
    check("huge_timeout_nowait_eagain", r == -EAGAIN, r);
    struct k_sembuf inc = { 0, 2, 0 };
    r = semtimedop(id, &inc, 1, &huge);
    check("huge_timeout_possible_ok", r == 0 && sem_val(id, 0) == 2, r);
    sem_rm(id);
    return finish();
}
