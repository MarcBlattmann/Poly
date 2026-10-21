/* Argument widths: semid, semnum and cmd are 32-bit ints (the upper half
 * of each register is ignored); for SETVAL the value is the low 32 bits
 * of the argument taken as a signed int (2^32+5 sets 5, 2^32-1 is -1 and
 * so ERANGE); for pointer commands the whole 64-bit argument is the
 * address.
 * poly-diff: syscalls=names (set ids depend on the IPC namespace's history)
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
int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    i64 hi = 1L << 32;
    i64 r = semctl(id | hi, 0, SETVAL, 3);
    check("semid_upper_ignored", r == 0, r);
    r = semctl(id, 1 | hi, SETVAL, 4);
    check("semnum_upper_ignored", r == 0, r);
    r = semctl(id, 1, GETVAL | hi, 0);
    check("cmd_upper_ignored", r == 4, r);
    r = semctl(id, 0, GETVAL, 0);
    check("value0", r == 3, r);
    r = semctl(id, 0, SETVAL, hi + 5);
    check("setval_low_32", r == 0, r);
    r = semctl(id, 0, GETVAL, 0);
    check("value_5", r == 5, r);
    r = semctl(id, 0, SETVAL, hi - 1);
    check("setval_0xffffffff_erange", r == -ERANGE, r);
    r = semctl(id, 0, SETVAL, (i64)0x8000000000000007UL);
    check("setval_sign_bit_ignored", r == 0, r);
    r = semctl(id, 0, GETVAL, 0);
    check("value_7", r == 7, r);
    struct k_semid_ds ds;
    r = semctl(id, 0, IPC_STAT, (i64)&ds | (1L << 47));
    check("pointer_full_width_efault", r == -EFAULT, r);
    r = semctl(id | (0x7fL << 40), -1L & ~0xffffffffL, IPC_STAT | (3L << 33), (i64)&ds);
    check("stat_upper_ignored", r == 0 && ds.nsems == 2, r);
    check("rm_ok", sem_rm(id | hi) == 0, 0);
    return finish();
}
