/* Order of the argument checks with a timeout. An unreadable timeout
 * (EFAULT) is reported before everything else: nsops 0, nsops > 500,
 * NULL sops, a negative or unknown id. Then nsops 0 (EINVAL) and
 * nsops > 500 (E2BIG), then an unreadable sops array (EFAULT), all
 * before an invalid timeout; an invalid timeout (EINVAL) beats an
 * unknown id, a bad sem_num (EFBIG), ERANGE and EAGAIN. With a valid
 * timeout the semop order holds: EFBIG, then per operation ERANGE or
 * EAGAIN.
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

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);
    i64 gone = semget(IPC_PRIVATE, 1, 0600);
    sem_rm(gone);
    struct k_sembuf up = { 0, 1, 0 };
    const struct k_timespec *unreadable = (const struct k_timespec *)8;

    check("efault_ts_vs_nsops0", semtimedop(id, &up, 0, unreadable) == -EFAULT, 0);
    check("efault_ts_vs_e2big", semtimedop(id, &up, 501, unreadable) == -EFAULT, 0);
    check("efault_ts_vs_null_sops", semtimedop(id, 0, 1, unreadable) == -EFAULT, 0);
    check("efault_ts_vs_neg_id", semtimedop(-1, &up, 1, unreadable) == -EFAULT, 0);
    check("efault_ts_vs_gone_id", semtimedop(gone, &up, 1, unreadable) == -EFAULT, 0);

    struct k_timespec inval = { 0, 1000000000 };
    i64 r = semtimedop(id, &up, 501, &inval);
    check("e2big_vs_bad_ts", r == -E2BIG, r);
    r = semtimedop(id, 0, 1, &inval);
    check("efault_sops_vs_bad_ts", r == -EFAULT, r);
    r = semtimedop(id, &up, 0, &inval);
    check("nsops0_with_bad_ts", r == -EINVAL, r);
    r = semtimedop(gone, &up, 1, &inval);
    check("bad_ts_vs_gone_id", r == -EINVAL, r);
    struct k_sembuf big = { 9, 1, 0 };
    r = semtimedop(id, &big, 1, &inval);
    check("bad_ts_vs_efbig", r == -EINVAL, r);
    sem_set(id, 0, 32767);
    r = ts_op1(id, 0, 1, 0, &inval);
    check("bad_ts_vs_erange", r == -EINVAL, r);
    r = ts_op1(id, 1, -1, IPC_NOWAIT, &inval);
    check("bad_ts_vs_eagain", r == -EINVAL, r);

    struct k_timespec t = { 0, 0 };
    r = semtimedop(id, &up, 0, &t);
    check("nsops0_einval", r == -EINVAL, r);
    r = semtimedop(id, 0, 501, &t);
    check("e2big", r == -E2BIG, r);
    r = semtimedop(gone, 0, 1, &t);
    check("efault_sops_vs_gone_id", r == -EFAULT, r);
    r = semtimedop(gone, &up, 1, &t);
    check("gone_id_einval", r == -EINVAL, r);
    r = semtimedop(-5, &up, 1, &t);
    check("neg_id_einval", r == -EINVAL, r);
    struct k_sembuf mixed[2] = { { 1, -1, 0 }, { 9, 1, 0 } };
    r = semtimedop(id, mixed, 2, &t);
    check("efbig_beats_eagain", r == -EFBIG, r);
    struct k_sembuf er[2] = { { 0, 1, 0 }, { 1, -1, 0 } };
    r = semtimedop(id, er, 2, &t);
    check("erange_first", r == -ERANGE, r);
    struct k_sembuf ea[2] = { { 1, -1, 0 }, { 0, 1, 0 } };
    r = semtimedop(id, ea, 2, &t);
    check("eagain_first", r == -EAGAIN, r);
    t.tv_sec = 5;
    r = ts_op1(id, 1, -1, IPC_NOWAIT, &t);
    check("nowait_ignores_timeout", r == -EAGAIN, r);
    r = ts_op1(id, 0, 1, 0, &t);
    check("erange_with_timeout", r == -ERANGE, r);
    check("values_kept", sem_val(id, 0) == 32767 && sem_val(id, 1) == 0, sem_val(id, 0));

    sem_rm(id);
    return finish();
}
