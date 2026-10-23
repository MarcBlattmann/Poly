/* Argument widths and results with a timeout: only the low 32 bits of
 * semid and nsops count (nsops 2^32+1 is one operation, 2^32 is 0:
 * EINVAL); sem_flg bits other than IPC_NOWAIT and SEM_UNDO are ignored;
 * 500 operations are accepted, 501 are E2BIG. A successful call sets
 * sempid of every semaphore it names (a sem_op 0 too) to the caller
 * and sem_otime to the current time; sem_ctime does not change.
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

/* One operation with a timeout (NULL: no timeout). */
static __attribute__((unused)) i64 ts_op1(i64 id, int num, int op, int flg, const struct k_timespec *ts) {
    struct k_sembuf b = { (unsigned short)num, (short)op, (short)flg };
    return semtimedop(id, &b, 1, ts);
}
static __attribute__((unused)) i64 elapsed_ms(i64 t0) { return (now_ns() - t0) / 1000000; }

static struct k_sembuf many[501];

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("semget_ok", id >= 0, id);
    struct k_semid_ds ds;
    sem_stat(id, &ds);
    i64 ctime0 = ds.ctime;
    struct k_timespec t = { 2, 0 };

    struct k_sembuf up = { 0, 1, 0 };
    i64 r = semtimedop(id, &up, (1UL << 32) + 1, &t);
    check("nsops_low32_one", r == 0 && sem_val(id, 0) == 1, r);
    r = semtimedop(id, &up, 1UL << 32, &t);
    check("nsops_low32_zero", r == -EINVAL, r);
    r = semtimedop(id | (1L << 32), &up, 1, &t);
    check("semid_low32", r == 0 && sem_val(id, 0) == 2, r);
    struct k_sembuf odd = { 0, -1, (short)0x2fff & ~(IPC_NOWAIT | SEM_UNDO) };
    r = semtimedop(id, &odd, 1, &t);
    check("other_flags_ignored", r == 0 && sem_val(id, 0) == 1, r);

    for (int i = 0; i < 501; i++) {
        many[i].num = 1;
        many[i].op = (i & 1) ? -1 : 1;
        many[i].flg = 0;
    }
    r = semtimedop(id, many, 500, &t);
    check("500_ops_ok", r == 0 && sem_val(id, 1) == 0, r);
    r = semtimedop(id, many, 501, &t);
    check("501_ops_e2big", r == -E2BIG, r);

    struct k_sembuf named[2] = { { 0, -1, 0 }, { 2, 0, 0 } };
    i64 before = now_sec();
    r = semtimedop(id, named, 2, &t);
    check("named_ok", r == 0 && sem_val(id, 0) == 0, r);
    i64 me = getpid();
    check("sempid_0", semctl(id, 0, GETPID, 0) == me, semctl(id, 0, GETPID, 0));
    check("sempid_2_zero_op", semctl(id, 2, GETPID, 0) == me, semctl(id, 2, GETPID, 0));
    sem_stat(id, &ds);
    check("otime_now", ds.otime >= before && ds.otime <= now_sec(), ds.otime);
    check("ctime_kept", ds.ctime == ctime0, ds.ctime);

    sem_rm(id);
    return finish();
}
