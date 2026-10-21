/* SETVAL and SETALL clear every process's pending SEM_UNDO adjustment
 * for the semaphores they set: a child that raised a semaphore with
 * SEM_UNDO and exits after the parent's SETVAL leaves the value the
 * parent set; without SETVAL the exit undoes the raise; SETVAL of one
 * semaphore leaves the adjustment of another in place.
 * poly-diff: syscalls=off (a child and the parent hand off by sleeping)
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
/* A child that applies `ops`, tells the parent through *gate, waits for
 * the parent to open *go, and exits. */
static volatile unsigned *gate, *go;
static i64 raiser(i64 id, struct k_sembuf *ops, int n) {
    *gate = 0;
    *go = 0;
    i64 pid = fork();
    if (pid == 0) {
        i64 r = semop(id, ops, n);
        *gate = r == 0 ? 1 : 2;
        while (!*go) sleep_ms(5);
        syscall1(SYS_exit, 0);
    }
    while (!*gate) sleep_ms(5);
    return pid;
}
int test_main(void) {
    volatile unsigned *sh = shared_page(4096);
    gate = sh;
    go = sh + 1;
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    semctl(id, 0, SETVAL, 1);
    struct k_sembuf up0 = { 0, 2, SEM_UNDO };
    i64 pid = raiser(id, &up0, 1);
    check("child_raised", *gate == 1 && semctl(id, 0, GETVAL, 0) == 3, semctl(id, 0, GETVAL, 0));
    semctl(id, 0, SETVAL, 10);
    *go = 1;
    wait_exit(pid);
    i64 r = semctl(id, 0, GETVAL, 0);
    check("setval_cleared_undo", r == 10, r);

    pid = raiser(id, &up0, 1);
    *go = 1;
    wait_exit(pid);
    r = semctl(id, 0, GETVAL, 0);
    check("exit_undoes_raise", r == 10, r);

    struct k_sembuf up1 = { 1, 3, SEM_UNDO };
    pid = raiser(id, &up1, 1);
    unsigned short v[2] = { 7, 8 };
    semctl(id, 0, SETALL, (i64)v);
    *go = 1;
    wait_exit(pid);
    r = semctl(id, 1, GETVAL, 0);
    check("setall_cleared_undo", r == 8, r);

    struct k_sembuf both[2] = { { 0, 1, SEM_UNDO }, { 1, 1, SEM_UNDO } };
    pid = raiser(id, both, 2);
    semctl(id, 0, SETVAL, 20);
    *go = 1;
    wait_exit(pid);
    r = semctl(id, 0, GETVAL, 0);
    check("set_sem_keeps_value", r == 20, r);
    r = semctl(id, 1, GETVAL, 0);
    check("other_sem_undone", r == 8, r);
    check("rm_ok", sem_rm(id) == 0, 0);
    return finish();
}
