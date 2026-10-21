/* Bad pointers: IPC_STAT, IPC_SET, GETALL, SETALL, IPC_INFO and
 * SEM_INFO with a NULL or unmapped argument are EFAULT; so is an array
 * that runs into an unmapped page; a failed SETALL changes no value and
 * a failed IPC_SET changes no field.
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
static void all_efault(i64 id, i64 p, const char *n_stat, const char *n_set, const char *n_get, const char *n_put,
                       const char *n_info, const char *n_seminfo) {
    i64 r = semctl(id, 0, IPC_STAT, p);
    check(n_stat, r == -EFAULT, r);
    r = semctl(id, 0, IPC_SET, p);
    check(n_set, r == -EFAULT, r);
    r = semctl(id, 0, GETALL, p);
    check(n_get, r == -EFAULT, r);
    r = semctl(id, 0, SETALL, p);
    check(n_put, r == -EFAULT, r);
    r = semctl(0, 0, IPC_INFO, p);
    check(n_info, r == -EFAULT, r);
    r = semctl(0, 0, SEM_INFO, p);
    check(n_seminfo, r == -EFAULT, r);
}
int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    semctl(id, 0, SETVAL, 3);
    semctl(id, 1, SETVAL, 4);
    all_efault(id, 0, "null_ipc_stat", "null_ipc_set", "null_getall", "null_setall", "null_ipc_info", "null_sem_info");
    all_efault(id, 1, "low_ipc_stat", "low_ipc_set", "low_getall", "low_setall", "low_ipc_info", "low_sem_info");
    i64 r = semctl(id, 0, GETVAL, 0);
    check("value0_kept", r == 3, r);
    r = semctl(id, 1, GETVAL, 0);
    check("value1_kept", r == 4, r);
    struct k_semid_ds ds;
    sem_stat(id, &ds);
    check("mode_kept", ds.mode == 0600, ds.mode);
    /* Two values whose second one lies in an unmapped page. */
    unsigned char *page = (unsigned char *)syscall6(SYS_mmap, 0, 8192, PROT_READ | PROT_WRITE,
                                                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, page + 4096, 4096);
    unsigned short *edge = (unsigned short *)(page + 4096 - 2);
    edge[0] = 9;
    r = semctl(id, 0, SETALL, (i64)edge);
    check("setall_straddling_efault", r == -EFAULT, r);
    r = semctl(id, 0, GETVAL, 0);
    check("straddle_value0_kept", r == 3, r);
    r = semctl(id, 0, GETALL, (i64)edge);
    check("getall_straddling_efault", r == -EFAULT, r);
    check("rm_ok", sem_rm(id) == 0, 0);
    return finish();
}
