/* GETNCNT and GETZCNT count the processes blocked in semop on one
 * semaphore: waiting for it to grow (a decrement that cannot proceed)
 * and waiting for it to be zero. SETVAL wakes waiters whose operation can
 * now proceed (the counts drop, the value reflects their operation);
 * GETPID then names the last process that changed the value.
 * poly-diff: syscalls=off (children block and wake; timing depends on scheduling)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_semget 64
#define SYS_semop 65
#define SYS_semctl 66
/* linux/ipc.h, linux/sem.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define IPC_64 0x0100
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
/* asm-generic/errno.h */
#define ERANGE 34
#define EIDRM 43
/* The largest semaphore value (SEMVMX, linux/sem.h). */
#define SEMVMX 32767

/* struct semid64_ds (asm/sembuf.h for x86-64, struct ipc64_perm from
 * asm-generic/ipcbuf.h), 104 bytes. */
struct k_semid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    i64 otime;
    u64 unused3;
    i64 ctime;
    u64 unused4;
    u64 nsems;
    u64 unused5, unused6;
};

/* struct seminfo (linux/sem.h), 40 bytes. */
struct k_seminfo {
    int semmap, semmni, semmns, semmnu, semmsl, semopm, semume, semusz, semvmx, semaem;
};

/* struct sembuf (linux/sem.h) */
struct k_sembuf {
    unsigned short num;
    short op;
    short flg;
};

static __attribute__((unused)) i64 semget(i64 key, i64 nsems, i64 flg) { return syscall3(SYS_semget, key, nsems, flg); }
static __attribute__((unused)) i64 semctl(i64 id, i64 num, i64 cmd, i64 arg) { return syscall4(SYS_semctl, id, num, cmd, arg); }
static __attribute__((unused)) i64 semop(i64 id, struct k_sembuf *ops, i64 n) { return syscall3(SYS_semop, id, ops, n); }
static __attribute__((unused)) i64 sem_rm(i64 id) { return semctl(id, 0, IPC_RMID, 0); }
static __attribute__((unused)) i64 sem_stat(i64 id, struct k_semid_ds *ds) {
    memset(ds, 0xa5, sizeof *ds);
    return semctl(id, 0, IPC_STAT, (i64)ds);
}
/* Set owner, group and mode with IPC_SET. */
static __attribute__((unused)) i64 sem_set(i64 id, unsigned uid, unsigned gid, unsigned mode) {
    struct k_semid_ds ds;
    memset(&ds, 0, sizeof ds);
    ds.uid = uid;
    ds.gid = gid;
    ds.mode = (unsigned short)mode;
    return semctl(id, 0, IPC_SET, (i64)&ds);
}
static __attribute__((unused)) i64 wall_seconds(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
static i64 waiter(i64 id, unsigned short num, short op) {
    i64 pid = fork();
    if (pid == 0) {
        struct k_sembuf b = { num, op, 0 };
        i64 r = semop(id, &b, 1);
        syscall1(SYS_exit, r == 0 ? 0 : r == -EIDRM ? 43 : 1);
    }
    return pid;
}
/* Poll until GETNCNT/GETZCNT of `num` reach the wanted values. */
static int wait_counts(i64 id, int num, i64 ncnt, i64 zcnt) {
    for (int i = 0; i < 500; i++) {
        if (semctl(id, num, GETNCNT, 0) == ncnt && semctl(id, num, GETZCNT, 0) == zcnt) return 1;
        sleep_ms(10);
    }
    return 0;
}
int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    i64 r = semctl(id, 0, GETNCNT, 0);
    check("ncnt_idle_zero", r == 0, r);
    r = semctl(id, 1, GETZCNT, 0);
    check("zcnt_idle_zero", r == 0, r);
    r = semctl(id, 2, GETNCNT, 0);
    check("ncnt_semnum_einval", r == -EINVAL, r);
    r = semctl(id, -1, GETZCNT, 0);
    check("zcnt_semnum_einval", r == -EINVAL, r);
    semctl(id, 1, SETVAL, 1);
    i64 a = waiter(id, 0, -1);
    i64 c = waiter(id, 0, -3);
    i64 z = waiter(id, 1, 0);
    check("two_decrementers", wait_counts(id, 0, 2, 0), semctl(id, 0, GETNCNT, 0));
    check("one_zero_waiter", wait_counts(id, 1, 0, 1), semctl(id, 1, GETZCNT, 0));
    r = semctl(id, 0, SETVAL, 1);
    check("setval_1_ok", r == 0, r);
    int sa = wait_exit(a);
    check("minus1_waiter_woken", sa == 0, sa);
    check("one_left", wait_counts(id, 0, 1, 0), semctl(id, 0, GETNCNT, 0));
    r = semctl(id, 0, GETVAL, 0);
    check("value_consumed", r == 0, r);
    r = semctl(id, 0, GETPID, 0);
    check("getpid_is_waker_child", r == a, r == getpid());
    r = semctl(id, 1, SETVAL, 0);
    check("setval_zero_ok", r == 0, r);
    int sz = wait_exit(z);
    check("zero_waiter_woken", sz == 0, sz);
    check("zcnt_back_to_zero", wait_counts(id, 1, 0, 0), semctl(id, 1, GETZCNT, 0));
    r = semctl(id, 0, SETVAL, 2);
    check("setval_2_ok", r == 0, r);
    sleep_ms(50);
    r = semctl(id, 0, GETNCNT, 0);
    check("minus3_still_blocked", r == 1, r);
    r = semctl(id, 0, SETVAL, 3);
    int sc = wait_exit(c);
    check("minus3_woken", sc == 0, sc);
    r = semctl(id, 0, GETVAL, 0);
    check("value_after_minus3", r == 0, r);
    check("rm_ok", sem_rm(id) == 0, 0);
    return finish();
}
