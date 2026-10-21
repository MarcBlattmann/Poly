/* SEM_UNDO at process exit: each semaphore gets the exiting process's
 * adjustment added back (5, -2 undo, +3 undo, -1 plain: 4 after exit).
 * The result is clamped to 0..32767 instead of failing. The exit counts
 * as an operation of the exiting process (sempid is its pid). SETVAL, by
 * the process itself or by another one, clears every process's
 * adjustment for that semaphore. A fork child does not inherit its
 * parent's adjustments, so the child's exit undoes nothing of the
 * parent's.
 * poly-diff: syscalls=off (several processes)
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
    i64 id = semget(IPC_PRIVATE, 1, 0600);
    check("semget_ok", id >= 0, id);

    sem_set(id, 0, 5);
    i64 pid = fork();
    if (pid == 0) {
        sem_op1(id, 0, -2, SEM_UNDO);
        sem_op1(id, 0, 3, SEM_UNDO);
        sem_op1(id, 0, -1, 0);
        exit_group(sem_val(id, 0) == 5 ? 0 : 1);
    }
    check("child_ok", wait_exit(pid) == 0, 0);
    check("undone_at_exit", sem_val(id, 0) == 4, sem_val(id, 0));
    check("exit_sets_sempid", semctl(id, 0, GETPID, 0) == pid, semctl(id, 0, GETPID, 0));

    sem_set(id, 0, 0);
    pid = fork();
    if (pid == 0) {
        sem_op1(id, 0, 5, SEM_UNDO);
        sem_op1(id, 0, -4, 0);
        exit_group(0);
    }
    wait_exit(pid);
    check("clamped_at_0", sem_val(id, 0) == 0, sem_val(id, 0));

    pid = fork();
    if (pid == 0) {
        sem_op1(id, 0, 10, 0);
        sem_op1(id, 0, -10, SEM_UNDO);
        sem_op1(id, 0, 32767, 0);
        exit_group(0);
    }
    wait_exit(pid);
    check("clamped_at_32767", sem_val(id, 0) == 32767, sem_val(id, 0));

    sem_set(id, 0, 0);
    pid = fork();
    if (pid == 0) {
        sem_op1(id, 0, 5, SEM_UNDO);
        sem_set(id, 0, 7);
        exit_group(0);
    }
    wait_exit(pid);
    check("own_setval_clears", sem_val(id, 0) == 7, sem_val(id, 0));

    sem_set(id, 0, 0);
    volatile unsigned *g = shared_page(4096);
    pid = fork();
    if (pid == 0) {
        sem_op1(id, 0, 5, SEM_UNDO);
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    sem_set(id, 0, 9);
    gate_open(&g[1]);
    wait_exit(pid);
    check("other_setval_clears", sem_val(id, 0) == 9, sem_val(id, 0));

    sem_set(id, 0, 0);
    i64 r = sem_op1(id, 0, 3, SEM_UNDO);
    check("parent_undo_op", r == 0, r);
    pid = fork();
    if (pid == 0) exit_group(0);
    wait_exit(pid);
    check("not_inherited", sem_val(id, 0) == 3, sem_val(id, 0));
    pid = fork();
    if (pid == 0) exit_group(sem_op1(id, 0, -3, SEM_UNDO | IPC_NOWAIT) == 0 ? 0 : 1);
    check("child_own_undo", wait_exit(pid) == 0, 0);
    check("child_undo_restores", sem_val(id, 0) == 3, sem_val(id, 0));

    sem_rm(id);
    return finish();
}
