/* Permissions of semctl for a caller that is not the owner (a child
 * that dropped to uid/gid 65534, no capabilities): the reading commands
 * (GETVAL, GETPID, GETNCNT, GETZCNT, GETALL, IPC_STAT, SEM_STAT) need r
 * in the "other" bits, SETVAL and SETALL need w (alter), SEM_STAT_ANY
 * needs nothing, IPC_SET and IPC_RMID need the caller to be the owner
 * (uid) or creator (cuid), else EPERM, whatever the mode. Order: GETVAL
 * checks access before semnum (EACCES); SETVAL checks semnum before
 * access (EINVAL). Once root gives the set to uid 65534 with IPC_SET,
 * that user is the owner: owner bits apply and IPC_RMID works.
 * poly-diff: needs=root (setresuid to another user; root creates the sets)
 * poly-diff: syscalls=off (several processes)
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

static int child_checks(i64 a, i64 b, i64 c) {
    int bad = 0;
    struct k_semid_ds ds;
    unsigned short v[1];
    if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) return 100;
    if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) return 101;
    /* a: mode 0600 owned by root. */
    if (sem_val(a, 0) != -EACCES) bad |= 1 << 0;
    if (semctl(a, 0, GETPID, 0) != -EACCES) bad |= 1 << 1;
    if (semctl(a, 0, GETNCNT, 0) != -EACCES) bad |= 1 << 1;
    if (semctl(a, 0, GETALL, (u64)v) != -EACCES) bad |= 1 << 2;
    if (sem_stat(a, &ds) != -EACCES) bad |= 1 << 3;
    if (semctl(a & 0x7fff, 0, SEM_STAT, (u64)&ds) != -EACCES) bad |= 1 << 4;
    if (semctl(a & 0x7fff, 0, SEM_STAT_ANY, (u64)&ds) != a) bad |= 1 << 5;
    if (sem_set(a, 0, 1) != -EACCES) bad |= 1 << 6;
    if (semctl(a, 5, GETVAL, 0) != -EACCES) bad |= 1 << 7;
    if (semctl(a, 5, SETVAL, 1) != -EINVAL) bad |= 1 << 8;
    if (sem_rm(a) != -EPERM) bad |= 1 << 9;
    memset(&ds, 0, sizeof ds);
    ds.mode = 0666;
    if (semctl(a, 0, IPC_SET, (u64)&ds) != -EPERM) bad |= 1 << 10;
    /* b: mode 0644. */
    if (sem_val(b, 0) != 3) bad |= 1 << 11;
    if (sem_stat(b, &ds) != 0) bad |= 1 << 12;
    if (sem_set(b, 0, 1) != -EACCES) bad |= 1 << 13;
    unsigned short w[1] = { 1 };
    if (semctl(b, 0, SETALL, (u64)w) != -EACCES) bad |= 1 << 14;
    if (semctl(b, 0, SETALL, 0) != -EACCES) bad |= 1 << 14;
    if (semctl(a, 0, GETALL, 0) != -EACCES) bad |= 1 << 2;
    unsigned short big[1] = { 40000 };
    if (semctl(b, 0, SETALL, (u64)big) != -EACCES) bad |= 1 << 14;
    /* c: mode 0666, but still not ours to remove. */
    if (sem_set(c, 0, 9) != 0) bad |= 1 << 15;
    if (sem_rm(c) != -EPERM) bad |= 1 << 16;
    return bad;
}

int test_main(void) {
    i64 a = semget(IPC_PRIVATE, 1, 0600);
    i64 b = semget(IPC_PRIVATE, 1, 0644);
    i64 c = semget(IPC_PRIVATE, 1, 0666);
    check("semget_ok", a >= 0 && b >= 0 && c >= 0, a);
    sem_set(b, 0, 3);
    i64 pid = fork();
    if (pid == 0) exit_group(child_checks(a, b, c));
    i64 st = wait_exit(pid);
    check("other_user_checks", st == 0, st);
    check("c_value_set_by_other", sem_val(c, 0) == 9, sem_val(c, 0));

    /* Give a to 65534 (mode 0600): it is now the owner. */
    struct k_semid_ds ds;
    sem_stat(a, &ds);
    ds.uid = 65534;
    ds.gid = 65534;
    i64 r = semctl(a, 0, IPC_SET, (u64)&ds);
    check("give_away_ok", r == 0, r);
    sem_stat(a, &ds);
    check("cuid_root_kept", ds.cuid == 0 && ds.uid == 65534, ds.uid);
    pid = fork();
    if (pid == 0) {
        if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) exit_group(100);
        if (syscall3(SYS_setresuid, 65534, 65534, 65534) != 0) exit_group(101);
        if (sem_set(a, 0, 4) != 0) exit_group(1);
        if (sem_val(a, 0) != 4) exit_group(2);
        exit_group(sem_rm(a) == 0 ? 0 : 3);
    }
    st = wait_exit(pid);
    check("new_owner_uses_and_removes", st == 0, st);
    r = sem_val(a, 0);
    check("removed_by_new_owner", r == -EINVAL, r);
    sem_rm(b);
    sem_rm(c);
    return finish();
}
