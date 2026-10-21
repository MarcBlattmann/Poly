/* nsems below 0 or above semmsl (32000 by default) is EINVAL before the
 * key is looked at: an unused key without IPC_CREAT is EINVAL, not ENOENT,
 * and a key in use with IPC_CREAT|IPC_EXCL is EINVAL, not EEXIST. A new set
 * of exactly semmsl semaphores is fine.
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
    i64 r = semget(key, -1, 0);
    check("unused_neg_einval", r == -EINVAL, r);
    r = semget(key, SEMMSL + 1, 0);
    check("unused_over_semmsl_einval", r == -EINVAL, r);
    r = semget(key, 0x7fffffff, 0);
    check("unused_intmax_einval", r == -EINVAL, r);
    r = semget(key, -1, 0600 | IPC_CREAT);
    check("creat_neg_einval", r == -EINVAL, r);
    r = semget(key, SEMMSL + 1, 0600 | IPC_CREAT);
    check("creat_over_semmsl_einval", r == -EINVAL, r);
    r = semget(key, 1, 0);
    check("nothing_made", r == -ENOENT, r);

    i64 id = semget(key, 1, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    r = semget(key, -1, 0);
    check("existing_neg_einval", r == -EINVAL, r);
    r = semget(key, SEMMSL + 1, IPC_CREAT | IPC_EXCL);
    check("range_before_eexist", r == -EINVAL, r);
    r = semget(key, -5, IPC_CREAT | IPC_EXCL);
    check("neg_before_eexist", r == -EINVAL, r);
    sem_rm(id);

    id = semget(IPC_PRIVATE, SEMMSL, 0600);
    check("semmsl_ok", id >= 0, id);
    r = sem_stat(id, &ds);
    check("semmsl_nsems", r == 0 && ds.nsems == SEMMSL, (i64)ds.nsems);
    sem_rm(id);
    r = semget(IPC_PRIVATE, SEMMSL + 1, 0600);
    check("private_over_semmsl_einval", r == -EINVAL, r);
    return finish();
}
