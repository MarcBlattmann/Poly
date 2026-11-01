/* GETVAL/SETVAL and GETALL/SETALL. SETVAL takes the value from the low
 * 32 bits of arg, accepts 0..32767 and is ERANGE otherwise, checked
 * before the id; it sets the semaphore's sempid to the caller and the
 * set's sem_ctime. SETALL reads nsems unsigned shorts from arg: a value
 * above 32767 is ERANGE and nothing changes; on success every value is
 * set and every sempid is the caller. GETALL writes nsems unsigned
 * shorts. An unreadable/unwritable array is EFAULT. Returns 0, GETVAL
 * returns the value.
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
    i64 me = getpid();
    i64 id = semget(IPC_PRIVATE, 3, 0600);
    check("semget_ok", id >= 0, id);

    i64 r = sem_set(id, 1, 32767);
    check("setval_max_ok", r == 0, r);
    check("getval_max", sem_val(id, 1) == 32767, sem_val(id, 1));
    r = sem_set(id, 1, 32768);
    check("setval_32768_erange", r == -ERANGE, r);
    r = sem_set(id, 1, -1);
    check("setval_negative_erange", r == -ERANGE, r);
    check("value_unchanged", sem_val(id, 1) == 32767, sem_val(id, 1));
    r = semctl(id, 1, SETVAL, (1UL << 32) | 5);
    check("setval_low_32_bits", r == 0 && sem_val(id, 1) == 5, sem_val(id, 1));
    r = semctl(id, 1, SETVAL, 0x1ffffffffUL);
    check("setval_low_32_bits_negative_erange", r == -ERANGE, r);
    r = semctl(0x7ffffff0, 0, SETVAL, 40000);
    check("erange_before_bad_id", r == -ERANGE, r);
    r = semctl(0x7ffffff0, 0, SETVAL, 1);
    check("bad_id_einval", r == -EINVAL, r);
    check("setval_sempid_caller", semctl(id, 1, GETPID, 0) == me, 0);
    check("other_sempid_0", semctl(id, 0, GETPID, 0) == 0, semctl(id, 0, GETPID, 0));
    r = sem_set(id, 0, 0);
    check("setval_zero_ok", r == 0, r);
    check("setval_zero_sempid", semctl(id, 0, GETPID, 0) == me, 0);

    unsigned short vals[4] = { 0xdead, 0xdead, 0xdead, 0xdead };
    r = semctl(id, 0, GETALL, (u64)vals);
    check("getall_ok", r == 0, r);
    check("getall_values", vals[0] == 0 && vals[1] == 5 && vals[2] == 0, vals[1]);
    check("getall_writes_nsems_only", vals[3] == 0xdead, vals[3]);

    unsigned short set[3] = { 7, 8, 32767 };
    r = semctl(id, 99, SETALL, (u64)set);
    check("setall_ok", r == 0, r);
    check("setall_v0", sem_val(id, 0) == 7, sem_val(id, 0));
    check("setall_v1", sem_val(id, 1) == 8, sem_val(id, 1));
    check("setall_v2", sem_val(id, 2) == 32767, sem_val(id, 2));
    check("setall_sempid", semctl(id, 2, GETPID, 0) == me, 0);
    unsigned short bad[3] = { 1, 32768, 2 };
    r = semctl(id, 0, SETALL, (u64)bad);
    check("setall_32768_erange", r == -ERANGE, r);
    check("setall_nothing_changed", sem_val(id, 0) == 7 && sem_val(id, 2) == 32767, sem_val(id, 0));
    unsigned short big[3] = { 65535, 0, 0 };
    r = semctl(id, 0, SETALL, (u64)big);
    check("setall_65535_erange", r == -ERANGE, r);

    r = semctl(id, 0, GETALL, 0);
    check("getall_null_efault", r == -EFAULT, r);
    r = semctl(id, 0, SETALL, 0);
    check("setall_null_efault", r == -EFAULT, r);
    r = semctl(id, 0, GETALL, 0xffff800000000000UL);
    check("getall_kernel_addr_efault", r == -EFAULT, r);
    check("values_kept_after_efault", sem_val(id, 1) == 8, sem_val(id, 1));

    /* GETVAL ignores arg. */
    r = semctl(id, 1, GETVAL, 0x1234);
    check("getval_ignores_arg", r == 8, r);
    sem_rm(id);
    return finish();
}
