/* Bad commands and the order of errors: unknown commands (4..10, 21,
 * 99, and any command with IPC_64 0x100 set) are EINVAL; a negative
 * semid is EINVAL before anything else; IPC_SET copies the struct in
 * before looking up the id (EFAULT for a removed id with a bad
 * pointer), IPC_STAT, GETALL and SETALL look up first (EINVAL); SETVAL
 * checks the value (ERANGE) before the id, semnum and permissions;
 * SETALL and GETALL check permissions before touching the buffer;
 * GETVAL checks permissions before semnum, SETVAL semnum before
 * permissions.
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
int test_main(void) {
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    i64 r;
    int bad_ok = 1;
    i64 bad_got = 0;
    for (int c = 4; c <= 10; c++) {
        r = semctl(id, 0, c, 0);
        if (r != -EINVAL) bad_ok = 0, bad_got = c;
    }
    check("cmds_4_to_10_einval", bad_ok, bad_got);
    r = semctl(id, 0, 21, 0);
    check("cmd_21_einval", r == -EINVAL, r);
    r = semctl(id, 0, 99, 0);
    check("cmd_99_einval", r == -EINVAL, r);
    r = semctl(id, 0, -1, 0);
    check("cmd_minus1_einval", r == -EINVAL, r);
    r = semctl(id, 0, GETVAL | IPC_64, 0);
    check("getval_ipc64_einval", r == -EINVAL, r);
    struct k_semid_ds ds;
    r = semctl(id, 0, IPC_STAT | IPC_64, (i64)&ds);
    check("stat_ipc64_einval", r == -EINVAL, r);

    r = semctl(-1, 0, IPC_SET, 1);
    check("neg_id_before_efault", r == -EINVAL, r);
    r = semctl(-1, 0, SETVAL, 40000);
    check("neg_id_before_erange", r == -EINVAL, r);
    r = semctl(-1, 0, IPC_INFO, 0);
    check("neg_id_info_einval", r == -EINVAL, r);

    i64 gone = semget(IPC_PRIVATE, 2, 0600);
    sem_rm(gone);
    r = semctl(gone, 0, IPC_SET, 1);
    check("ipc_set_efault_before_lookup", r == -EFAULT, r);
    r = semctl(gone, 0, IPC_STAT, 1);
    check("ipc_stat_lookup_before_efault", r == -EINVAL, r);
    r = semctl(gone, 0, GETALL, 1);
    check("getall_lookup_before_efault", r == -EINVAL, r);
    r = semctl(gone, 0, SETALL, 1);
    check("setall_lookup_before_efault", r == -EINVAL, r);
    r = semctl(gone, 0, SETVAL, 40000);
    check("setval_erange_before_lookup", r == -ERANGE, r);

    i64 z = semget(IPC_PRIVATE, 2, 0);
    unsigned short big[2] = { 1, 40000 };
    r = semctl(z, 0, SETVAL, 40000);
    check("setval_erange_before_eacces", r == -ERANGE, r);
    r = semctl(z, 9, SETVAL, 1);
    check("setval_semnum_before_eacces", r == -EINVAL, r);
    r = semctl(z, 9, GETVAL, 0);
    check("getval_eacces_before_semnum", r == -EACCES, r);
    r = semctl(z, 0, SETALL, 1);
    check("setall_eacces_before_efault", r == -EACCES, r);
    r = semctl(z, 0, SETALL, (i64)big);
    check("setall_eacces_before_erange", r == -EACCES, r);
    r = semctl(z, 0, GETALL, 1);
    check("getall_eacces_before_efault", r == -EACCES, r);
    r = semctl(z, 0, IPC_STAT, 1);
    check("stat_eacces_before_efault", r == -EACCES, r);
    check("rm_ok", sem_rm(id) == 0 && sem_rm(z) == 0, 0);
    return finish();
}
