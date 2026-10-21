/* Who gets a semaphore: sleepers asking for the same amount are served
 * in the order they went to sleep. A later sleeper whose request can be
 * met is not held back by an earlier one whose request cannot (-2
 * waiting, -1 waiting, +1: the -1 sleeper proceeds). One increment can
 * wake several sleepers (+3 with two -1 sleepers leaves 1). A decrement
 * to zero wakes a wait-for-zero sleeper.
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

static int has_exited(i64 pid) {
    int st;
    for (int i = 0; i < 200; i++) {
        if (syscall4(SYS_wait4, pid, &st, WNOHANG, 0) == pid) return 1;
        sleep_ms(1);
    }
    return 0;
}

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 1, 0600);
    check("semget_ok", id >= 0, id);

    i64 a = fork();
    if (a == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    check("a_sleeps", wait_count(id, 0, GETNCNT, 1), 0);
    i64 b = fork();
    if (b == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    check("b_sleeps", wait_count(id, 0, GETNCNT, 2), 0);
    sem_op1(id, 0, 1, 0);
    check("first_sleeper_served", has_exited(a), 0);
    check("second_still_sleeping", semctl(id, 0, GETNCNT, 0) == 1, semctl(id, 0, GETNCNT, 0));
    sem_op1(id, 0, 1, 0);
    check("second_served", has_exited(b), 0);

    a = fork();
    if (a == 0) exit_group(sem_op1(id, 0, -2, 0) == 0 ? 0 : 1);
    wait_count(id, 0, GETNCNT, 1);
    b = fork();
    if (b == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    check("both_sleep", wait_count(id, 0, GETNCNT, 2), 0);
    sem_op1(id, 0, 1, 0);
    check("small_later_served", has_exited(b), 0);
    check("big_still_sleeping", semctl(id, 0, GETNCNT, 0) == 1 && sem_val(id, 0) == 0, semctl(id, 0, GETNCNT, 0));
    sem_op1(id, 0, 2, 0);
    check("big_served", has_exited(a), 0);
    check("value_0", sem_val(id, 0) == 0, sem_val(id, 0));

    a = fork();
    if (a == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    b = fork();
    if (b == 0) exit_group(sem_op1(id, 0, -1, 0) == 0 ? 0 : 1);
    check("two_sleep", wait_count(id, 0, GETNCNT, 2), 0);
    sem_op1(id, 0, 3, 0);
    check("one_left", sem_val(id, 0) == 1, sem_val(id, 0));
    check("ncnt_0", semctl(id, 0, GETNCNT, 0) == 0, semctl(id, 0, GETNCNT, 0));
    check("both_woken", wait_exit(a) == 0 && wait_exit(b) == 0, 0);

    a = fork();
    if (a == 0) exit_group(sem_op1(id, 0, 0, 0) == 0 ? 0 : 1);
    check("zero_sleeper", wait_count(id, 0, GETZCNT, 1), 0);
    i64 r = sem_op1(id, 0, -1, IPC_NOWAIT);
    check("down_to_zero", r == 0, r);
    check("zcnt_0", semctl(id, 0, GETZCNT, 0) == 0, semctl(id, 0, GETZCNT, 0));
    check("zero_sleeper_ok", wait_exit(a) == 0, 0);

    sem_rm(id);
    return finish();
}
