/* Set ids are non-negative ints, distinct among live sets, and not file
 * descriptors: making sets takes no fd slot (the next pipe still gets 3
 * and 4). An id just removed is not handed out by the next semget.
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
    i64 ids[8];
    int ok = 1, distinct = 1;
    for (int i = 0; i < 8; i++) {
        ids[i] = semget(IPC_PRIVATE, 1, 0600);
        if (ids[i] < 0 || ids[i] > 0x7fffffff) ok = 0;
        for (int j = 0; j < i; j++)
            if (ids[j] == ids[i]) distinct = 0;
    }
    check("ids_non_negative", ok, ids[0]);
    check("ids_distinct", distinct, 0);
    int fds[2] = { -1, -1 };
    i64 r = syscall1(SYS_pipe, fds);
    check("no_fd_slots_taken", r == 0 && fds[0] == 3 && fds[1] == 4, fds[0]);
    close(fds[0]);
    close(fds[1]);
    int gone = 1;
    for (int i = 0; i < 8; i++)
        if (sem_rm(ids[i]) != 0) gone = 0;
    check("all_removed", gone, 0);
    int fresh = 1;
    i64 prev = semget(IPC_PRIVATE, 1, 0600);
    for (int i = 0; i < 8; i++) {
        sem_rm(prev);
        i64 next = semget(IPC_PRIVATE, 1, 0600);
        if (next < 0 || next == prev) fresh = 0;
        prev = next;
    }
    sem_rm(prev);
    check("removed_id_not_reused_next", fresh, prev);
    int key = own_key(1);
    i64 a = semget(key, 1, 0600 | IPC_CREAT);
    i64 b = semget(IPC_PRIVATE, 1, 0600);
    check("keyed_and_private_distinct", a >= 0 && b >= 0 && a != b, b);
    sem_rm(a);
    sem_rm(b);
    return finish();
}
