/* SETALL and GETALL: the argument points to nsems unsigned shorts;
 * GETALL writes exactly nsems of them (nothing past the end); semnum is
 * ignored; SETALL with any value above 32767 is ERANGE and changes no
 * semaphore; a 400-semaphore set round-trips.
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
static unsigned short big[401];
int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("create_ok", id >= 0, id);
    unsigned short out[4] = { 9, 9, 9, 9 };
    i64 r = semctl(id, 0, GETALL, (i64)out);
    check("getall_new_ok", r == 0, r);
    check("getall_new_zeros", out[0] == 0 && out[1] == 0 && out[2] == 0, out[0] + out[1] + out[2]);
    check("getall_stops_at_nsems", out[3] == 9, out[3]);
    unsigned short in[3] = { 1, 2, SEMVMX };
    r = semctl(id, 0, SETALL, (i64)in);
    check("setall_ok", r == 0, r);
    check("setall_val0", semctl(id, 0, GETVAL, 0) == 1, semctl(id, 0, GETVAL, 0));
    check("setall_val1", semctl(id, 1, GETVAL, 0) == 2, semctl(id, 1, GETVAL, 0));
    check("setall_val2", semctl(id, 2, GETVAL, 0) == SEMVMX, semctl(id, 2, GETVAL, 0));
    memset(out, 0xff, sizeof out);
    out[3] = 9;
    r = semctl(id, 77, GETALL, (i64)out);
    check("getall_semnum_ignored", r == 0, r);
    check("getall_values", out[0] == 1 && out[1] == 2 && out[2] == SEMVMX && out[3] == 9, out[2]);
    unsigned short in2[3] = { 4, 5, 6 };
    r = semctl(id, -5, SETALL, (i64)in2);
    check("setall_semnum_ignored", r == 0, r);
    check("setall_semnum_ignored_val", semctl(id, 2, GETVAL, 0) == 6, semctl(id, 2, GETVAL, 0));
    unsigned short bad[3] = { 10, 11, SEMVMX + 1 };
    r = semctl(id, 0, SETALL, (i64)bad);
    check("setall_32768_erange", r == -ERANGE, r);
    unsigned short bad2[3] = { 65535, 11, 12 };
    r = semctl(id, 0, SETALL, (i64)bad2);
    check("setall_65535_erange", r == -ERANGE, r);
    memset(out, 0, sizeof out);
    semctl(id, 0, GETALL, (i64)out);
    check("erange_changed_nothing", out[0] == 4 && out[1] == 5 && out[2] == 6, out[0]);
    check("rm_ok", sem_rm(id) == 0, 0);

    i64 id2 = semget(IPC_PRIVATE, 400, 0600);
    check("create_400_ok", id2 >= 0, id2);
    for (int i = 0; i < 400; i++) big[i] = (unsigned short)(i * 7);
    r = semctl(id2, 0, SETALL, (i64)big);
    check("setall_400_ok", r == 0, r);
    r = semctl(id2, 399, GETVAL, 0);
    check("getval_399", r == 399 * 7, r);
    memset(big, 0, sizeof big);
    big[400] = 0x1234;
    r = semctl(id2, 0, GETALL, (i64)big);
    check("getall_400_ok", r == 0, r);
    int same = 1;
    for (int i = 0; i < 400; i++)
        if (big[i] != (unsigned short)(i * 7)) same = 0;
    check("getall_400_values", same, 0);
    check("getall_400_stops", big[400] == 0x1234, big[400]);
    check("rm_400_ok", sem_rm(id2) == 0, 0);
    return finish();
}
