/* IPC_RMID removes a set at once: its key is free again (lookup is
 * ENOENT), IPC_CREAT makes a new set with a different id, and the old
 * id is EINVAL for semctl. The new set starts fresh (values 0).
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
    struct k_semid_ds ds;
    int key = own_key(1);
    i64 id = semget(key, 2, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 r = semctl(id, 1, SETVAL, 5);
    check("setval_ok", r == 0, r);
    r = sem_rm(id);
    check("rmid_ok", r == 0, r);
    r = semget(key, 1, 0);
    check("removed_key_enoent", r == -ENOENT, r);
    r = sem_stat(id, &ds);
    check("old_id_stat_einval", r == -EINVAL, r);
    r = semctl(id, 1, GETVAL, 0);
    check("old_id_getval_einval", r == -EINVAL, r);
    i64 id2 = semget(key, 2, 0600 | IPC_CREAT | IPC_EXCL);
    check("recreate_ok", id2 >= 0, id2);
    check("recreate_new_id", id2 != id, id2);
    r = semctl(id2, 1, GETVAL, 0);
    check("recreate_fresh_value", r == 0, r);
    r = semget(key, 0, 0);
    check("lookup_new_set", r == id2, r);
    r = sem_rm(id2);
    check("rmid2_ok", r == 0, r);
    r = sem_rm(id2);
    check("rmid_twice_einval", r == -EINVAL, r);
    return finish();
}
