/* GETPID: 0 for a semaphore nothing has changed yet; SETVAL makes it the
 * caller's pid for that semaphore only; SETALL for every semaphore;
 * reads (GETVAL, GETALL, IPC_STAT) do not change it; a child that sets a
 * value is reported by its pid, also after it has exited.
 * poly-diff: syscalls=off (pids differ between runs; fork)
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
    i64 me = getpid();
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("create_ok", id >= 0, id);
    i64 r = semctl(id, 0, GETPID, 0);
    check("fresh_zero", r == 0, r);
    semctl(id, 1, SETVAL, 5);
    r = semctl(id, 1, GETPID, 0);
    check("setval_sets_caller", r == me, r == 0);
    r = semctl(id, 0, GETPID, 0);
    check("setval_only_that_sem", r == 0, r);
    r = semctl(id, 3, GETPID, 0);
    check("semnum_einval", r == -EINVAL, r);
    i64 pid = fork();
    if (pid == 0) {
        i64 v = semctl(id, 1, GETVAL, 0);
        semctl(id, 2, SETVAL, 6);
        syscall1(SYS_exit, v == 5 ? 0 : 1);
    }
    int s = wait_exit(pid);
    check("child_ok", s == 0, s);
    r = semctl(id, 2, GETPID, 0);
    check("child_pid_reported", r == pid, r == me);
    r = semctl(id, 1, GETPID, 0);
    check("child_read_kept_pid", r == me, r == pid);
    unsigned short v[3] = { 1, 2, 3 };
    semctl(id, 0, SETALL, (i64)v);
    int all = 1;
    for (int i = 0; i < 3; i++)
        if (semctl(id, i, GETPID, 0) != me) all = 0;
    check("setall_sets_all", all, 0);
    pid = fork();
    if (pid == 0) {
        unsigned short w[3];
        struct k_semid_ds ds;
        semctl(id, 0, GETALL, (i64)w);
        semctl(id, 0, GETVAL, 0);
        sem_stat(id, &ds);
        semctl(id, 0, SETVAL, 40000);
        syscall1(SYS_exit, 0);
    }
    wait_exit(pid);
    r = semctl(id, 0, GETPID, 0);
    check("reads_and_failed_set_keep_pid", r == me, r == pid);
    check("rm_ok", sem_rm(id) == 0, 0);
    return finish();
}
