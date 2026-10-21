/* Access: a call whose operations are all wait-for-zero (sem_op 0) needs
 * read permission (r); a call with any non-zero sem_op needs alter
 * permission (w) and then nothing else, even if it also has zero
 * operations. Only the caller's class counts (the owner here): group or
 * other bits do not help the owner, and x grants nothing. Missing
 * access: EACCES, checked before any operation is tried (EACCES wins
 * over EAGAIN), and nothing is applied. A caller with CAP_IPC_OWNER
 * (root) passes every check. (semctl GETVAL needs read permission
 * too, so values are looked at with the mode back at 0600.)
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

static i64 set_mode(i64 id, int mode) {
    struct k_semid_ds ds;
    sem_stat(id, &ds);
    ds.mode = (unsigned short)mode;
    return semctl(id, 0, IPC_SET, (u64)&ds);
}

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    i64 denied = root ? 0 : -EACCES;
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);
    sem_set(id, 0, 3);

    check("chmod_0400", set_mode(id, 0400) == 0, 0);
    i64 r = sem_op1(id, 1, 0, 0);
    check("r_zero_op_ok", r == 0, r);
    r = sem_op1(id, 0, 0, IPC_NOWAIT);
    check("r_zero_op_eagain", r == -EAGAIN, r);
    r = sem_op1(id, 0, 1, 0);
    check("r_add_denied", r == denied, r);
    r = sem_op1(id, 1, -1, IPC_NOWAIT);
    check("r_sub_eacces_before_eagain", r == (root ? -EAGAIN : -EACCES), r);
    struct k_sembuf mixed[2] = { { 1, 0, 0 }, { 0, -1, 0 } };
    r = semop(id, mixed, 2);
    check("r_mixed_denied", r == denied, r);
    check("r_value", sem_val(id, 0) == 3, sem_val(id, 0));

    check("chmod_0200", set_mode(id, 0200) == 0, 0);
    r = sem_op1(id, 1, 0, 0);
    check("w_zero_op_denied", r == denied, r);
    r = sem_op1(id, 0, 1, 0);
    check("w_add_ok", r == 0, r);
    r = sem_op1(id, 0, -2, 0);
    check("w_sub_ok", r == 0, r);
    struct k_sembuf mixed2[2] = { { 1, 0, 0 }, { 0, 1, 0 } };
    r = semop(id, mixed2, 2);
    check("w_mixed_ok", r == 0, r);
    /* GETVAL needs read permission too: look with the mode back at 0600. */
    set_mode(id, 0600);
    check("w_value", sem_val(id, 0) == 3, sem_val(id, 0));

    check("chmod_0000", set_mode(id, 0) == 0, 0);
    r = sem_op1(id, 1, 0, 0);
    check("none_zero_op_denied", r == denied, r);
    r = sem_op1(id, 0, 1, 0);
    check("none_add_denied", r == denied, r);
    set_mode(id, 0600);
    check("none_value", sem_val(id, 0) == (root ? 4 : 3), sem_val(id, 0));

    check("chmod_0066", set_mode(id, 0066) == 0, 0);
    r = sem_op1(id, 0, 1, 0);
    check("group_other_w_not_owner", r == denied, r);
    r = sem_op1(id, 1, 0, 0);
    check("group_other_r_not_owner", r == denied, r);

    check("chmod_0100", set_mode(id, 0100) == 0, 0);
    r = sem_op1(id, 0, 1, 0);
    check("x_grants_no_alter", r == denied, r);
    r = sem_op1(id, 1, 0, 0);
    check("x_grants_no_read", r == denied, r);

    check("rm_ok", sem_rm(id) == 0, 0);
    return finish();
}
