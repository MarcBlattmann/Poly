/* Sets are system-wide, not per process: a child makes a keyed set,
 * sets a value and exits; the set outlives it, the parent finds it by
 * key with the child's value, and IPC_STAT shows the child's set. A set
 * made before fork is found by key in the child too, and a value the
 * child sets is seen by the parent.
 * poly-diff: syscalls=off (two processes)
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
#define IPC_STAT 2
#define GETPID 11
#define GETVAL 12
#define GETALL 13
#define GETNCNT 14
#define GETZCNT 15
#define SETVAL 16
/* The default semmsl (most semaphores in one set). */
#define SEMMSL 32000

/* struct semid64_ds (asm/sembuf.h for x86-64, struct ipc64_perm from
 * asm-generic/ipcbuf.h). */
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

/* struct sembuf (linux/sem.h) */
struct k_sembuf {
    unsigned short num;
    short op;
    short flg;
};

static __attribute__((unused)) i64 semget(i64 key, i64 nsems, i64 flg) { return syscall3(SYS_semget, key, nsems, flg); }
static __attribute__((unused)) i64 semctl(i64 id, i64 num, i64 cmd, i64 arg) { return syscall4(SYS_semctl, id, num, cmd, arg); }
static __attribute__((unused)) i64 sem_rm(i64 id) { return semctl(id, 0, IPC_RMID, 0); }
static __attribute__((unused)) i64 sem_stat(i64 id, struct k_semid_ds *ds) {
    memset(ds, 0xa5, sizeof *ds);
    return semctl(id, 0, IPC_STAT, (i64)ds);
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x50000000 | (tag << 22) | (int)getpid()); }
int test_main(void) {
    struct k_semid_ds ds;
    int key = own_key(1);
    i64 pid = fork();
    if (pid == 0) {
        i64 id = semget(key, 3, 0600 | IPC_CREAT | IPC_EXCL);
        if (id < 0) exit_group(1);
        if (semctl(id, 2, SETVAL, 42) != 0) exit_group(2);
        exit_group(0);
    }
    check("child_ok", wait_exit(pid) == 0, 0);
    i64 id = semget(key, 0, 0600);
    check("set_outlives_child", id >= 0, id);
    i64 r = sem_stat(id, &ds);
    check("stat_ok", r == 0 && ds.nsems == 3 && ds.mode == 0600, (i64)ds.nsems);
    r = semctl(id, 2, GETVAL, 0);
    check("child_value_visible", r == 42, r);
    r = semctl(id, 2, GETPID, 0);
    check("child_is_last_pid", r == pid, r);
    sem_rm(id);

    int key2 = own_key(2);
    i64 id2 = semget(key2, 1, 0600 | IPC_CREAT | IPC_EXCL);
    check("parent_create_ok", id2 >= 0, id2);
    pid = fork();
    if (pid == 0) {
        if (semget(key2, 0, 0) != id2) exit_group(3);
        if (semctl(id2, 0, SETVAL, 7) != 0) exit_group(4);
        exit_group(0);
    }
    check("child_finds_parent_set", wait_exit(pid) == 0, 0);
    r = semctl(id2, 0, GETVAL, 0);
    check("parent_sees_child_value", r == 7, r);
    sem_rm(id2);
    return finish();
}
