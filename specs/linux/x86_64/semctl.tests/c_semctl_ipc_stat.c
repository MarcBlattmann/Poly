/* IPC_STAT fills the 104-byte struct semid64_ds and nothing past it:
 * key, owner and creator ids, mode, nsems, otime 0 before any semop,
 * ctime within the creation second, reserved words 0; semnum is ignored;
 * the same data for every call; a keyed set shows its key.
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
static __attribute__((unused)) int own_key(int tag) { return (int)(0x51000000 | (tag << 22) | (int)getpid()); }
int test_main(void) {
    unsigned euid = (unsigned)syscall0(SYS_geteuid), egid = (unsigned)syscall0(SYS_getegid);
    int key = own_key(1);
    i64 t0 = wall_seconds();
    i64 id = semget(key, 5, 0640 | IPC_CREAT | IPC_EXCL);
    i64 t1 = wall_seconds();
    check("create_ok", id >= 0, id);
    unsigned char buf[160];
    memset(buf, 0xa5, sizeof buf);
    i64 r = semctl(id, 0, IPC_STAT, (i64)buf);
    check("stat_ok", r == 0, r);
    int last = 159;
    while (last >= 0 && buf[last] == 0xa5) last--;
    check("writes_104_bytes", last < 104, last);
    int past = 1;
    for (int i = 104; i < 160; i++)
        if (buf[i] != 0xa5) past = 0;
    check("nothing_past_104", past, 0);
    struct k_semid_ds ds;
    memcpy(&ds, buf, sizeof ds);
    check("key", ds.key == key, ds.key);
    check("uid", ds.uid == euid, ds.uid);
    check("gid", ds.gid == egid, ds.gid);
    check("cuid", ds.cuid == euid, ds.cuid);
    check("cgid", ds.cgid == egid, ds.cgid);
    check("mode", ds.mode == 0640, ds.mode);
    check("nsems", ds.nsems == 5, (i64)ds.nsems);
    check("otime_zero", ds.otime == 0, ds.otime);
    check("ctime_now", ds.ctime >= t0 && ds.ctime <= t1, ds.ctime - t0);
    check("pads_zero", ds.pad1 == 0 && ds.pad2 == 0 && ds.pad3 == 0, ds.pad1);
    check("reserved_zero", ds.unused1 == 0 && ds.unused2 == 0 && ds.unused3 == 0 && ds.unused4 == 0 &&
          ds.unused5 == 0 && ds.unused6 == 0, (i64)ds.unused3);
    struct k_semid_ds ds2;
    memset(&ds2, 0x5a, sizeof ds2);
    r = semctl(id, -9, IPC_STAT, (i64)&ds2);
    check("semnum_ignored", r == 0, r);
    check("same_data", mem_eq(&ds, &ds2, sizeof ds), 0);
    semctl(id, 1, SETVAL, 3);
    r = semctl(id, 0, IPC_STAT, (i64)&ds2);
    check("setval_keeps_otime_zero", r == 0 && ds2.otime == 0, ds2.otime);
    check("rm_ok", sem_rm(id) == 0, 0);

    i64 p = semget(IPC_PRIVATE, 1, 0600);
    r = sem_stat(p, &ds);
    check("private_stat_ok", r == 0, r);
    check("private_key_zero", ds.key == 0, ds.key);
    check("private_nsems", ds.nsems == 1, (i64)ds.nsems);
    check("rm_private_ok", sem_rm(p) == 0, 0);
    return finish();
}
