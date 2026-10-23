/* Removing the set while callers sleep in semtimedop ends their calls
 * at once with EIDRM, long before their timeouts, whatever kind of
 * operation they wait for. Afterwards the id is EINVAL, also with a
 * timeout.
 * poly-diff: syscalls=off (several processes, timing-dependent polling)
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
    volatile i64 *sh = shared_page(4096);
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);
    sem_set(id, 1, 1);

    i64 a = fork();
    if (a == 0) {
        struct k_timespec t = { 20, 0 };
        i64 t0 = now_ns();
        sh[0] = ts_op1(id, 0, -1, 0, &t);
        sh[1] = elapsed_ms(t0);
        exit_group(0);
    }
    i64 b = fork();
    if (b == 0) {
        struct k_timespec t = { 20, 0 };
        sh[2] = ts_op1(id, 1, 0, 0, &t);
        exit_group(0);
    }
    check("ncnt_1", wait_count(id, 0, GETNCNT, 1), 0);
    check("zcnt_1", wait_count(id, 1, GETZCNT, 1), 0);
    check("rm_ok", sem_rm(id) == 0, 0);
    check("a_exit", wait_exit(a) == 0, 0);
    check("b_exit", wait_exit(b) == 0, 0);
    check("decrement_eidrm", sh[0] == -EIDRM, sh[0]);
    check("eidrm_early", sh[1] < 10000, sh[1]);
    check("zero_wait_eidrm", sh[2] == -EIDRM, sh[2]);

    struct k_timespec t = { 1, 0 };
    i64 r = ts_op1(id, 0, 1, 0, &t);
    check("removed_id_einval", r == -EINVAL, r);
    return finish();
}
