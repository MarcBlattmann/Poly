/* Finding a used key checks the access asked for in the low 9 bits of
 * semflg: the r, w and x bits of all three classes are merged and each
 * merged bit must be granted by the set's mode for the caller's class
 * (the owner here). Missing: EACCES. Asking for nothing (0) always
 * works. A caller with CAP_IPC_OWNER (root) passes every check. Creating
 * asks for nothing, so a mode 0 set can be made. EINVAL (nsems larger
 * than the set) is reported before EACCES.
 * poly-diff: syscalls=names (semaphore ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_semget 64
#define SYS_semop 65
#define SYS_semctl 66
#define SYS_semtimedop 220
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define E2BIG 7
#define EFBIG 27
#define ENOSPC 28
#define ERANGE 34
#define EIDRM 43
/* linux/ipc.h, linux/sem.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
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

/* struct sembuf (linux/sem.h) */
struct k_sembuf {
    unsigned short num;
    short op;
    short flg;
};

/* struct semid64_ds (arch/x86/include/uapi/asm/sembuf.h, struct
 * ipc64_perm from asm-generic/ipcbuf.h) on x86-64: 104 bytes. */
struct k_semid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    i64 otime;
    u64 otime_high;
    i64 ctime;
    u64 ctime_high;
    u64 nsems;
    u64 unused3, unused4;
};

static __attribute__((unused)) i64 semget(i64 key, i64 nsems, i64 flg) { return syscall3(SYS_semget, key, nsems, flg); }
static __attribute__((unused)) i64 semop(i64 id, const void *sops, u64 n) { return syscall3(SYS_semop, id, sops, n); }
static __attribute__((unused)) i64 semtimedop(i64 id, const void *sops, u64 n, const void *ts) {
    return syscall4(SYS_semtimedop, id, sops, n, ts);
}
static __attribute__((unused)) i64 semctl(i64 id, i64 num, i64 cmd, u64 arg) { return syscall4(SYS_semctl, id, num, cmd, arg); }
static __attribute__((unused)) i64 sem_rm(i64 id) { return semctl(id, 0, IPC_RMID, 0); }
static __attribute__((unused)) i64 sem_val(i64 id, int n) { return semctl(id, n, GETVAL, 0); }
static __attribute__((unused)) i64 sem_set(i64 id, int n, int v) { return semctl(id, n, SETVAL, (u64)v); }
static __attribute__((unused)) i64 sem_stat(i64 id, struct k_semid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return semctl(id, 0, IPC_STAT, (u64)ds);
}
static __attribute__((unused)) i64 sem_op1(i64 id, int num, int op, int flg) {
    struct k_sembuf b = { (unsigned short)num, (short)op, (short)flg };
    return semop(id, &b, 1);
}
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }

int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    int key = own_key(1);
    i64 id = semget(key, 1, 0640 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 denied = root ? id : -EACCES;

    i64 r = semget(key, 1, 0);
    check("ask_nothing_ok", r == id, r);
    r = semget(key, 1, 0600);
    check("owner_rw_ok", r == id, r);
    r = semget(key, 1, 0044);
    check("group_other_r_bits_ok", r == id, r);
    r = semget(key, 1, 0022);
    check("group_other_w_bits_check_owner_w", r == id, r);
    r = semget(key, 1, 0100);
    check("owner_x_denied", r == denied, r);
    r = semget(key, 1, 0001);
    check("other_x_bit_denied", r == denied, r);
    r = semget(key, 1, 0700 | IPC_CREAT);
    check("creat_existing_checks_access", r == denied, r);
    r = semget(key, 2, 0100);
    check("nsems_before_access", r == -EINVAL, r);
    sem_rm(id);

    key = own_key(2);
    id = semget(key, 1, 0400 | IPC_CREAT | IPC_EXCL);
    check("ro_create_ok", id >= 0, id);
    denied = root ? id : -EACCES;
    r = semget(key, 1, 0444);
    check("ro_read_ok", r == id, r);
    r = semget(key, 1, 0200);
    check("ro_write_denied", r == denied, r);
    r = semget(key, 1, 0002);
    check("ro_other_w_bit_denied", r == denied, r);
    sem_rm(id);

    key = own_key(3);
    id = semget(key, 1, IPC_CREAT | IPC_EXCL);
    check("mode0_create_ok", id >= 0, id);
    denied = root ? id : -EACCES;
    r = semget(key, 1, 0);
    check("mode0_ask_nothing_ok", r == id, r);
    r = semget(key, 1, 0400);
    check("mode0_read_denied", r == denied, r);
    check("mode0_rm_by_owner_ok", sem_rm(id) == 0, 0);
    return finish();
}
