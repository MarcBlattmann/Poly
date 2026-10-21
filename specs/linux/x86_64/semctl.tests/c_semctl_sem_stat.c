/* SEM_STAT and SEM_STAT_ANY take an index (0 .. the value IPC_INFO
 * returns) instead of an id and return the id of the set in that slot,
 * filling the struct as IPC_STAT does; scanning the indexes finds every
 * live set exactly once. An empty slot, a negative index or one past the
 * highest is EINVAL. SEM_STAT needs read permission (EACCES); SEM_STAT_ANY
 * does not. The slot of a set is its id modulo 32768.
 * poly-diff: syscalls=names (indexes and ids depend on the IPC namespace's history)
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
static i64 find_index(i64 id, int cmd, struct k_semid_ds *out) {
    struct k_seminfo si;
    i64 max = semctl(0, 0, SEM_INFO, (i64)&si);
    i64 found = -1;
    int times = 0;
    for (i64 i = 0; i <= max; i++) {
        struct k_semid_ds ds;
        if (semctl(i, 0, cmd, (i64)&ds) == id) {
            if (found < 0) {
                found = i;
                *out = ds;
            }
            times++;
        }
    }
    return times == 1 ? found : -2;
}
int test_main(void) {
    i64 a = semget(IPC_PRIVATE, 3, 0640);
    i64 z = semget(IPC_PRIVATE, 2, 0);
    check("create_ok", a >= 0 && z >= 0, a);
    struct k_semid_ds st, ds;
    i64 ia = find_index(a, SEM_STAT, &ds);
    check("sem_stat_finds_once", ia >= 0, ia);
    sem_stat(a, &st);
    check("sem_stat_same_as_ipc_stat", ds.nsems == 3 && ds.mode == 0640 && ds.uid == st.uid && ds.ctime == st.ctime &&
          ds.seq == st.seq, (i64)ds.nsems);
    check("slot_is_id_mod_32768", ia == a % 32768, ia);
    i64 r = semctl(ia, 0, SEM_STAT_ANY, (i64)&ds);
    check("sem_stat_any_returns_id", r == a, r);
    i64 iz = find_index(z, SEM_STAT_ANY, &ds);
    check("sem_stat_any_finds_mode0", iz >= 0 && iz != ia, iz);
    check("sem_stat_any_fields", ds.nsems == 2 && ds.mode == 0, ds.mode);
    r = semctl(iz, 0, SEM_STAT, (i64)&ds);
    check("sem_stat_mode0_eacces", r == -EACCES, r);
    r = semctl(-1, 0, SEM_STAT_ANY, (i64)&ds);
    check("negative_index_einval", r == -EINVAL, r);
    r = semctl(ia, 0, SEM_STAT, 1);
    check("sem_stat_bad_ptr_efault", r == -EFAULT, r);
    struct k_seminfo si;
    i64 max = semctl(0, 0, SEM_INFO, (i64)&si);
    check("max_covers_sets", max >= ia && max >= iz, max);
    sem_rm(a);
    r = semctl(ia, 0, SEM_STAT_ANY, (i64)&ds);
    check("removed_slot_einval", r == -EINVAL || (r >= 0 && r != a), r);
    i64 again = find_index(a, SEM_STAT_ANY, &ds);
    check("removed_not_found", again == -2, again);
    r = sem_rm(z);
    check("rm_ok", r == 0, r);
    return finish();
}
