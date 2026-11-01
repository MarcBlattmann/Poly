/* semget(IPC_PRIVATE, ...) always makes a new set, whatever the flags
 * (IPC_CREAT not needed, IPC_EXCL ignored): a new id every call. The
 * new set: nsems as asked, mode = semflg & 0777 (other bits not kept),
 * uid = cuid = euid, gid = cgid = egid, sem_otime 0, sem_ctime now,
 * every value 0 and every sempid 0. Only the low 32 bits of key and
 * nsems count (key 1<<32 is IPC_PRIVATE, nsems (1<<32)|3 is 3).
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
    i64 t0 = now_sec();
    i64 a = semget(IPC_PRIVATE, 3, 0640);
    check("private_ok", a >= 0, a);
    i64 b = semget(IPC_PRIVATE, 3, 0640 | IPC_CREAT | IPC_EXCL);
    check("private_excl_ok", b >= 0, b);
    check("new_id", b != a, b);
    i64 c = semget(IPC_PRIVATE, 1, 0);
    check("private_no_flags_ok", c >= 0 && c != a && c != b, c);

    struct k_semid_ds ds;
    i64 r = sem_stat(a, &ds);
    check("stat_ok", r == 0, r);
    check("key_0", ds.key == 0, ds.key);
    check("nsems_3", ds.nsems == 3, (i64)ds.nsems);
    check("mode_0640", ds.mode == 0640, ds.mode);
    check("uid_euid", ds.uid == (unsigned)syscall0(SYS_geteuid), ds.uid);
    check("cuid_euid", ds.cuid == (unsigned)syscall0(SYS_geteuid), ds.cuid);
    check("gid_egid", ds.gid == (unsigned)syscall0(SYS_getegid), ds.gid);
    check("cgid_egid", ds.cgid == (unsigned)syscall0(SYS_getegid), ds.cgid);
    check("otime_0", ds.otime == 0, ds.otime);
    i64 t1 = now_sec();
    check("ctime_now", ds.ctime >= t0 && ds.ctime <= t1, ds.ctime - t0);
    int zero = 1;
    for (int i = 0; i < 3; i++)
        if (sem_val(a, i) != 0 || semctl(a, i, GETPID, 0) != 0) zero = 0;
    check("values_and_pids_0", zero, 0);

    /* Flag bits other than the permission bits are not stored. */
    i64 d = semget(IPC_PRIVATE, 1, 0777 | IPC_NOWAIT | 0x10000000 | SEM_UNDO);
    check("extra_bits_ok", d >= 0, d);
    sem_stat(d, &ds);
    check("extra_bits_mode_0777", ds.mode == 0777, ds.mode);

    /* High halves of key and nsems are ignored. */
    i64 e = semget(1L << 32, (1L << 32) | 3, 0600);
    check("high_bits_private_ok", e >= 0, e);
    sem_stat(e, &ds);
    check("high_bits_key_0", ds.key == 0, ds.key);
    check("high_bits_nsems_3", ds.nsems == 3, (i64)ds.nsems);

    sem_rm(a);
    sem_rm(b);
    sem_rm(c);
    sem_rm(d);
    sem_rm(e);
    return finish();
}
