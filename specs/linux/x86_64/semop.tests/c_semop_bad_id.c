/* The set id: a negative id, an id never handed out and the id of a
 * removed set are all EINVAL (a removed set is not EIDRM for a call
 * that does not block). Only the low 32 bits of the id register count.
 * The operations array is read before the id is looked up, so an
 * unreadable array is EFAULT even with a bad id.
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
    struct k_sembuf b = { 0, 1, 0 };
    i64 r = semop(-1, &b, 1);
    check("negative_einval", r == -EINVAL, r);
    r = semop(0x7ffffff0, &b, 1);
    check("unused_id_einval", r == -EINVAL, r);
    r = semop(-1, 0, 1);
    check("efault_before_id", r == -EFAULT, r);

    i64 id = semget(IPC_PRIVATE, 1, 0600);
    check("semget_ok", id >= 0, id);
    r = semop(id | (1L << 32), &b, 1);
    check("high_bits_ignored", r == 0, r);
    check("high_bits_applied", sem_val(id, 0) == 1, sem_val(id, 0));
    r = semop(id | (0xabcdL << 40), &b, 1);
    check("high_bits_ignored_2", r == 0 && sem_val(id, 0) == 2, r);

    check("rm_ok", sem_rm(id) == 0, 0);
    r = semop(id, &b, 1);
    check("removed_einval", r == -EINVAL, r);
    b.op = 0;
    b.flg = IPC_NOWAIT;
    r = semop(id, &b, 1);
    check("removed_zero_op_einval", r == -EINVAL, r);
    r = semop(id, 0, 1);
    check("removed_null_efault", r == -EFAULT, r);
    return finish();
}
