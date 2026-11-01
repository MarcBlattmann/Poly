/* A timed sleeper woken before its timeout returns 0, with its
 * operations applied by the waker (right after the waking semop the
 * value is already taken); a sleeper whose set is removed returns EIDRM
 * before its timeout.
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

static int wait_count(i64 id, int num, int cmd, i64 want) {
    for (int i = 0; i < 2000; i++) {
        if (semctl(id, num, cmd, 0) == want) return 1;
        sleep_ms(1);
    }
    return 0;
}

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 1, 0600);
    check("semget_ok", id >= 0, id);
    i64 pid = fork();
    if (pid == 0) {
        struct k_timespec ts = { 10, 0 };
        struct k_sembuf b = { 0, -1, 0 };
        i64 t0 = now_ns();
        i64 r = semtimedop(id, &b, 1, &ts);
        i64 ms = (now_ns() - t0) / 1000000;
        exit_group(r == 0 && ms < 5000 ? 0 : 1);
    }
    check("sleeping", wait_count(id, 0, GETNCNT, 1), 0);
    i64 r = sem_op1(id, 0, 1, 0);
    check("wake_ok", r == 0, r);
    check("taken_at_once", sem_val(id, 0) == 0, sem_val(id, 0));
    check("sleeper_returned_0_early", wait_exit(pid) == 0, 0);

    pid = fork();
    if (pid == 0) {
        struct k_timespec ts = { 10, 0 };
        struct k_sembuf b = { 0, -1, 0 };
        i64 t0 = now_ns();
        i64 e = semtimedop(id, &b, 1, &ts);
        i64 ms = (now_ns() - t0) / 1000000;
        exit_group(e == -EIDRM && ms < 5000 ? 0 : 1);
    }
    check("sleeping_2", wait_count(id, 0, GETNCNT, 1), 0);
    r = sem_rm(id);
    check("rm_ok", r == 0, r);
    check("sleeper_eidrm", wait_exit(pid) == 0, 0);
    return finish();
}
