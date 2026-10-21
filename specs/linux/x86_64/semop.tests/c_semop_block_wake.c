/* Blocking: without IPC_NOWAIT a decrement that cannot proceed sleeps
 * and counts in semncnt (GETNCNT), a wait-for-zero on a non-zero value
 * sleeps and counts in semzcnt (GETZCNT). When another process makes
 * the operation possible, it is completed for the sleeper before the
 * waking semop returns: right after the +1 the value is already taken
 * and the count is back to 0; sempid is the sleeper's pid. A sleeping
 * call with several operations applies none of them while it waits and
 * counts only on the semaphore of the first operation that cannot
 * proceed; when that one becomes possible it moves on to the next.
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

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);

    i64 pid = fork();
    if (pid == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    check("ncnt_1", wait_count(id, 0, GETNCNT, 1), 0);
    check("zcnt_0", semctl(id, 0, GETZCNT, 0) == 0, semctl(id, 0, GETZCNT, 0));
    i64 r = sem_op1(id, 0, 1, 0);
    check("wake_op_ok", r == 0, r);
    check("taken_at_once", sem_val(id, 0) == 0, sem_val(id, 0));
    check("ncnt_0_at_once", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    check("sleeper_ok", wait_exit(pid) == 0, 0);
    check("sempid_sleeper", semctl(id, 0, GETPID, 0) == pid, semctl(id, 0, GETPID, 0));

    sem_set(id, 0, 2);
    pid = fork();
    if (pid == 0) exit_group(sem_op1(id, 0, 0, 0) == 0 ? 0 : 1);
    check("zcnt_1", wait_count(id, 0, GETZCNT, 1), 0);
    check("zero_waiter_ncnt_0", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    sem_op1(id, 0, -1, 0);
    sleep_ms(20);
    check("still_waiting_at_1", semctl(id, 0, GETZCNT, 0) == 1, semctl(id, 0, GETZCNT, 0));
    sem_op1(id, 0, -1, 0);
    check("zcnt_0_at_once", semctl(id, 0, GETZCNT, 0) == 0, semctl(id, 0, GETZCNT, 0));
    check("zero_waiter_ok", wait_exit(pid) == 0, 0);

    /* Second operation blocks: the first is not applied meanwhile. */
    sem_set(id, 0, 1);
    sem_set(id, 1, 0);
    pid = fork();
    if (pid == 0) {
        struct k_sembuf b[2] = { { 0, -1, 0 }, { 1, -1, 0 } };
        exit_group(semop(id, b, 2) == 0 ? 0 : 1);
    }
    check("counted_on_sem1", wait_count(id, 1, GETNCNT, 1), 0);
    check("not_counted_on_sem0", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    check("first_not_applied", sem_val(id, 0) == 1, sem_val(id, 0));
    sem_op1(id, 1, 1, 0);
    check("multi_ok", wait_exit(pid) == 0, 0);
    check("both_applied", sem_val(id, 0) == 0 && sem_val(id, 1) == 0, sem_val(id, 0));

    /* First operation blocks: counted there, then on the second. */
    pid = fork();
    if (pid == 0) {
        struct k_sembuf b[2] = { { 0, -1, 0 }, { 1, -1, 0 } };
        exit_group(semop(id, b, 2) == 0 ? 0 : 1);
    }
    check("counted_on_first", wait_count(id, 0, GETNCNT, 1), 0);
    check("not_on_second", semctl(id, 1, GETNCNT, 0) == 0, semctl(id, 1, GETNCNT, 0));
    sem_op1(id, 0, 1, 0);
    check("moved_to_second", wait_count(id, 1, GETNCNT, 1), 0);
    check("left_first", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    check("first_value_kept", sem_val(id, 0) == 1, sem_val(id, 0));
    sem_op1(id, 1, 1, 0);
    check("multi2_ok", wait_exit(pid) == 0, 0);
    check("both_applied_2", sem_val(id, 0) == 0 && sem_val(id, 1) == 0, sem_val(id, 0));

    sem_rm(id);
    return finish();
}
