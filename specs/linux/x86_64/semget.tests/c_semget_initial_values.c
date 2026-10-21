/* Every semaphore of a new set starts at 0 with sempid 0 and no waiters:
 * GETALL reads zeros, GETVAL/GETPID/GETNCNT/GETZCNT read 0 for each index,
 * a decrement with IPC_NOWAIT is EAGAIN and a wait-for-zero returns at
 * once. Indexes run from 0 to nsems-1; nsems itself is EINVAL.
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
    i64 id = semget(IPC_PRIVATE, 5, 0600);
    check("create_ok", id >= 0, id);
    unsigned short vals[6] = { 9, 9, 9, 9, 9, 9 };
    i64 r = semctl(id, 0, GETALL, (i64)vals);
    check("getall_ok", r == 0, r);
    int zeros = 1;
    for (int i = 0; i < 5; i++)
        if (vals[i] != 0) zeros = 0;
    check("getall_zeros", zeros, vals[0]);
    check("getall_stops_at_nsems", vals[5] == 9, vals[5]);
    int bad = 0;
    for (int i = 0; i < 5; i++) {
        if (semctl(id, i, GETVAL, 0) != 0) bad |= 1;
        if (semctl(id, i, GETPID, 0) != 0) bad |= 2;
        if (semctl(id, i, GETNCNT, 0) != 0) bad |= 4;
        if (semctl(id, i, GETZCNT, 0) != 0) bad |= 8;
    }
    check("per_index_zero", bad == 0, bad);
    r = semctl(id, 5, GETVAL, 0);
    check("index_nsems_einval", r == -EINVAL, r);
    struct k_sembuf dec = { 2, -1, IPC_NOWAIT };
    r = syscall3(SYS_semop, id, &dec, 1);
    check("decrement_nowait_eagain", r == -EAGAIN, r);
    struct k_sembuf zero = { 4, 0, 0 };
    r = syscall3(SYS_semop, id, &zero, 1);
    check("wait_zero_immediate", r == 0, r);
    sem_rm(id);
    return finish();
}
