/* IPC_STAT fills a struct semid64_ds; IPC_SET takes uid, gid and the low
 * 9 bits of mode from one (other fields are ignored), keeps cuid/cgid,
 * nsems and the values, and sets sem_ctime; GETVAL etc. see the new mode
 * at once. A uid or gid of -1 is EINVAL and changes nothing. The owner
 * may give the set to any uid; as its creator (cuid) it keeps owner
 * rights. An unreadable buffer is EFAULT for IPC_SET, an unwritable one
 * for IPC_STAT. IPC_RMID ignores arg.
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
    unsigned uid = (unsigned)syscall0(SYS_geteuid), gid = (unsigned)syscall0(SYS_getegid);
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("semget_ok", id >= 0, id);
    sem_set(id, 1, 11);
    struct k_semid_ds ds, set;
    i64 r = sem_stat(id, &ds);
    check("stat_ok", r == 0, r);

    set = ds;
    set.mode = 0177777;
    set.uid = uid;
    set.gid = gid;
    set.cuid = 4242;
    set.nsems = 99;
    set.otime = 77;
    set.ctime = 77;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("set_ok", r == 0, r);
    sem_stat(id, &ds);
    check("set_mode_low_9_bits", ds.mode == 0777, ds.mode);
    check("set_cuid_kept", ds.cuid == uid, ds.cuid);
    check("set_nsems_kept", ds.nsems == 2, (i64)ds.nsems);
    check("set_otime_kept", ds.otime == 0, ds.otime);
    check("set_ctime_now", ds.ctime > 77, ds.ctime);
    check("set_values_kept", sem_val(id, 1) == 11, sem_val(id, 1));

    set.mode = 0400;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("set_ro_ok", r == 0, r);
    sem_stat(id, &ds);
    check("mode_0400", ds.mode == 0400, ds.mode);
    r = sem_set(id, 0, 1);
    check("setval_after_ro", r == (uid == 0 ? 0 : -EACCES), r);
    r = sem_val(id, 1);
    check("getval_after_ro", r == 11, r);

    set.mode = 0600;
    set.uid = 0xffffffffu;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("set_uid_minus1_einval", r == -EINVAL, r);
    set.uid = uid;
    set.gid = 0xffffffffu;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("set_gid_minus1_einval", r == -EINVAL, r);
    sem_stat(id, &ds);
    check("einval_changed_nothing", ds.mode == 0400 && ds.gid == gid, ds.mode);

    /* Give the set away: the creator keeps owner rights. */
    set.gid = gid;
    set.uid = 12345;
    set.mode = 0600;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("give_away_ok", r == 0, r);
    sem_stat(id, &ds);
    check("new_uid", ds.uid == 12345, ds.uid);
    check("cuid_still_me", ds.cuid == uid, ds.cuid);
    r = sem_val(id, 1);
    check("creator_reads_0600", r == 11, r);
    set.uid = uid;
    r = semctl(id, 0, IPC_SET, (u64)&set);
    check("creator_sets_back", r == 0, r);

    r = semctl(id, 0, IPC_SET, 0);
    check("set_null_efault", r == -EFAULT, r);
    r = semctl(id, 0, IPC_STAT, 0);
    check("stat_null_efault", r == -EFAULT, r);
    r = semctl(id, 0, IPC_RMID, 0x1234);
    check("rmid_ignores_arg", r == 0, r);
    r = sem_stat(id, &ds);
    check("stat_removed_einval", r == -EINVAL, r);
    return finish();
}
