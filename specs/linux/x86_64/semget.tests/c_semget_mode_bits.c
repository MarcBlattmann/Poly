/* A new set's mode is the low 9 bits of semflg. Other bits (IPC_NOWAIT,
 * 010000, unknown high bits, the upper half of the register) are accepted
 * and not stored.
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
static int mode_of(i64 id) {
    struct k_semid_ds ds;
    if (sem_stat(id, &ds) != 0) return -1;
    return ds.mode;
}

int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 1, 0777);
    check("mode_777", id >= 0 && mode_of(id) == 0777, mode_of(id));
    sem_rm(id);
    id = semget(IPC_PRIVATE, 1, 07777);
    check("setid_bits_dropped", id >= 0 && mode_of(id) == 0777, mode_of(id));
    sem_rm(id);
    id = semget(IPC_PRIVATE, 1, 0644 | IPC_NOWAIT | 010000 | 020000);
    check("flag_bits_dropped", id >= 0 && mode_of(id) == 0644, mode_of(id));
    sem_rm(id);
    id = semget(IPC_PRIVATE, 1, 0600 | 0x7ff00000);
    check("high_bits_accepted", id >= 0 && mode_of(id) == 0600, mode_of(id));
    sem_rm(id);
    id = semget(IPC_PRIVATE, 1, 0x7700000000L | 0444);
    check("upper_half_ignored", id >= 0 && mode_of(id) == 0444, mode_of(id));
    sem_rm(id);
    int key = own_key(1);
    id = semget(key, 1, 0460 | IPC_CREAT | IPC_EXCL | 040000);
    check("keyed_mode", id >= 0 && mode_of(id) == 0460, mode_of(id));
    i64 r = semget(key, 1, 0);
    check("keyed_lookup", r == id, r);
    sem_rm(id);
    return finish();
}
