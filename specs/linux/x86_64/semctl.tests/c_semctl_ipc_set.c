/* IPC_SET reads uid, gid and mode from the struct and nothing else: mode
 * keeps only its low 9 bits; the owner may hand the set to any uid and
 * gid and, as creator, still controls it; uid or gid (unsigned)-1 is
 * EINVAL and changes nothing; key, cuid, cgid, nsems and the times other
 * than ctime are unaffected; ctime becomes the current time.
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
    unsigned euid = (unsigned)syscall0(SYS_geteuid), egid = (unsigned)syscall0(SYS_getegid);
    i64 id = semget(IPC_PRIVATE, 2, 0600);
    check("create_ok", id >= 0, id);
    struct k_semid_ds before, ds;
    sem_stat(id, &before);
    sleep_ms(1100);
    struct k_semid_ds in;
    memset(&in, 0x5a, sizeof in);
    in.uid = euid;
    in.gid = egid;
    in.mode = 07640;
    i64 t0 = wall_seconds();
    i64 r = semctl(id, 0, IPC_SET, (i64)&in);
    i64 t1 = wall_seconds();
    check("set_ok", r == 0, r);
    r = sem_stat(id, &ds);
    check("stat_ok", r == 0, r);
    check("mode_low_9_bits", ds.mode == 0640, ds.mode);
    check("key_unchanged", ds.key == 0, ds.key);
    check("nsems_unchanged", ds.nsems == 2, (i64)ds.nsems);
    check("cuid_unchanged", ds.cuid == euid, ds.cuid);
    check("cgid_unchanged", ds.cgid == egid, ds.cgid);
    check("otime_unchanged", ds.otime == 0, ds.otime);
    check("ctime_updated", ds.ctime > before.ctime && ds.ctime >= t0 && ds.ctime <= t1, ds.ctime - before.ctime);
    check("seq_unchanged", ds.seq == before.seq, ds.seq);

    r = sem_set(id, 12345, 54321, 0660);
    check("give_away_ok", r == 0, r);
    sem_stat(id, &ds);
    check("uid_given", ds.uid == 12345, ds.uid);
    check("gid_given", ds.gid == 54321, ds.gid);
    check("cuid_kept", ds.cuid == euid, ds.cuid);
    check("mode_given", ds.mode == 0660, ds.mode);
    r = semctl(id, 0, SETVAL, 4);
    check("creator_still_owner_class", r == 0, r);
    r = sem_set(id, (unsigned)-1, egid, 0600);
    check("uid_minus1_einval", r == -EINVAL, r);
    r = sem_set(id, euid, (unsigned)-1, 0600);
    check("gid_minus1_einval", r == -EINVAL, r);
    sem_stat(id, &ds);
    check("einval_changed_nothing", ds.uid == 12345 && ds.gid == 54321 && ds.mode == 0660, ds.mode);
    r = sem_set(id, euid, egid, 0400);
    check("creator_may_set", r == 0, r);
    sem_stat(id, &ds);
    check("back_to_owner", ds.uid == euid && ds.gid == egid && ds.mode == 0400, ds.uid);
    r = semctl(id, 7, IPC_SET, (i64)&in);
    check("semnum_ignored", r == 0, r);
    r = semctl(id, 0, GETVAL, 0);
    check("value_kept", r == 4, r);
    r = sem_rm(id);
    check("rm_ok", r == 0, r);
    return finish();
}
