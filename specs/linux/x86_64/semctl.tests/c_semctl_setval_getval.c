/* SETVAL and GETVAL: a new semaphore reads 0; SETVAL stores 0..32767
 * (SEMVMX) and GETVAL reads it back; 32768 and negative values are
 * ERANGE and leave the value alone; semnum must be 0..nsems-1 (EINVAL
 * otherwise, also for a negative semnum); each semaphore is separate.
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
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("create_ok", id >= 0, id);
    i64 r = semctl(id, 0, GETVAL, 0);
    check("new_value_zero", r == 0, r);
    r = semctl(id, 2, GETVAL, 0);
    check("new_last_zero", r == 0, r);
    r = semctl(id, 1, SETVAL, 42);
    check("setval_ok", r == 0, r);
    r = semctl(id, 1, GETVAL, 0);
    check("getval_42", r == 42, r);
    r = semctl(id, 0, GETVAL, 0);
    check("neighbour_0_untouched", r == 0, r);
    r = semctl(id, 2, GETVAL, 0);
    check("neighbour_2_untouched", r == 0, r);
    r = semctl(id, 1, SETVAL, SEMVMX);
    check("setval_max_ok", r == 0, r);
    r = semctl(id, 1, GETVAL, 0);
    check("getval_max", r == SEMVMX, r);
    r = semctl(id, 1, SETVAL, 0);
    check("setval_zero_ok", r == 0, r);
    r = semctl(id, 1, GETVAL, 0);
    check("getval_zero", r == 0, r);
    semctl(id, 1, SETVAL, 7);
    r = semctl(id, 1, SETVAL, SEMVMX + 1);
    check("setval_32768_erange", r == -ERANGE, r);
    r = semctl(id, 1, SETVAL, -1);
    check("setval_minus1_erange", r == -ERANGE, r);
    r = semctl(id, 1, SETVAL, 65536);
    check("setval_65536_erange", r == -ERANGE, r);
    r = semctl(id, 1, GETVAL, 0);
    check("erange_kept_value", r == 7, r);
    r = semctl(id, 3, GETVAL, 0);
    check("getval_semnum_nsems_einval", r == -EINVAL, r);
    r = semctl(id, -1, GETVAL, 0);
    check("getval_semnum_neg_einval", r == -EINVAL, r);
    r = semctl(id, 3, SETVAL, 1);
    check("setval_semnum_nsems_einval", r == -EINVAL, r);
    r = semctl(id, -1, SETVAL, 1);
    check("setval_semnum_neg_einval", r == -EINVAL, r);
    r = semctl(id, 3, SETVAL, -1);
    check("erange_before_semnum", r == -ERANGE, r);
    r = sem_rm(id);
    check("rm_ok", r == 0, r);
    return finish();
}
