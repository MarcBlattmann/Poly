/* After a successful call, sempid (semctl GETPID) of every semaphore the
 * call names is the caller's pid, including semaphores named only by a
 * wait-for-zero operation; the others keep theirs (0 in a new set).
 * sem_otime (IPC_STAT) becomes the current time; it is 0 in a new set
 * and a failed call changes neither. semop does not change sem_ctime.
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
#define GETPID 11
#define GETVAL 12
#define GETALL 13
#define GETNCNT 14
#define GETZCNT 15
#define SETVAL 16
#define SETALL 17
#define SEM_UNDO 0x1000

/* struct sembuf (linux/sem.h) */
struct k_sembuf {
    unsigned short num;
    short op;
    short flg;
};

/* struct semid64_ds (asm-generic/sembuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64. */
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
/* One operation: semop with a single sembuf. */
static __attribute__((unused)) i64 sem_op1(i64 id, int num, int op, int flg) {
    struct k_sembuf b = { (unsigned short)num, (short)op, (short)flg };
    return semop(id, &b, 1);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* Poll a semctl count (GETNCNT, GETZCNT) until it is `want`; 0 after ~2 s. */
static __attribute__((unused)) int wait_count(i64 id, int num, int cmd, i64 want) {
    for (int i = 0; i < 2000; i++) {
        if (semctl(id, num, cmd, 0) == want) return 1;
        sleep_ms(1);
    }
    return 0;
}

int test_main(void) {
    struct k_semid_ds ds;
    i64 me = getpid();
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("semget_ok", id >= 0, id);
    sem_stat(id, &ds);
    check("new_otime_0", ds.otime == 0, ds.otime);
    check("new_sempid_0", semctl(id, 0, GETPID, 0) == 0, semctl(id, 0, GETPID, 0));
    i64 ctime0 = ds.ctime;

    i64 r = sem_op1(id, 7, 1, 0);
    check("efbig", r == -EFBIG, r);
    r = sem_op1(id, 2, -1, IPC_NOWAIT);
    check("failed_op", r == -EAGAIN, r);
    check("failed_no_sempid", semctl(id, 2, GETPID, 0) == 0, semctl(id, 2, GETPID, 0));
    sem_stat(id, &ds);
    check("failed_no_otime", ds.otime == 0, ds.otime);

    i64 before = now_sec();
    r = sem_op1(id, 2, 0, 0);
    i64 after = now_sec();
    check("zero_op_ok", r == 0, r);
    check("zero_op_sets_sempid", semctl(id, 2, GETPID, 0) == me, semctl(id, 2, GETPID, 0));
    check("other_sempid_0", semctl(id, 0, GETPID, 0) == 0, semctl(id, 0, GETPID, 0));
    sem_stat(id, &ds);
    check("otime_now", ds.otime >= before && ds.otime <= after, ds.otime);
    check("ctime_unchanged", ds.ctime == ctime0, ds.ctime);

    struct k_sembuf b[2] = { { 0, 1, 0 }, { 0, -1, 0 } };
    r = semop(id, b, 2);
    check("net_zero_ok", r == 0 && sem_val(id, 0) == 0, r);
    check("net_zero_sets_sempid", semctl(id, 0, GETPID, 0) == me, semctl(id, 0, GETPID, 0));

    sem_stat(id, &ds);
    i64 otime1 = ds.otime;
    r = sem_op1(id, 1, -9, IPC_NOWAIT);
    check("fail_again", r == -EAGAIN, r);
    sem_stat(id, &ds);
    check("failed_otime_kept", ds.otime == otime1, ds.otime);

    sem_rm(id);
    return finish();
}
