/* SHM_STAT and SHM_STAT_ANY take a slot index (only the low 15 bits of
 * the shmid argument count) and return the full id of the segment in
 * that slot, filling the buffer as IPC_STAT does. An empty slot is EINVAL;
 * a removed segment's slot is empty.
 * poly-diff: syscalls=names (segment ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
#define SYS_prlimit64 302
/* linux/ipc.h, linux/shm.h, asm-generic/resource.h, asm-generic/errno.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define IPC_64 0x0100
#define SHM_LOCK 11
#define SHM_UNLOCK 12
#define SHM_STAT 13
#define SHM_INFO 14
#define SHM_STAT_ANY 15
#define SHM_DEST 01000
#define SHM_LOCKED 02000
#define RLIMIT_MEMLOCK 8

/* struct shmid64_ds (asm-generic/shmbuf.h) with struct ipc64_perm
 * (asm-generic/ipcbuf.h) on x86-64: 112 bytes. */
struct k_shmid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned mode;
    unsigned short seq, pad2;
    u64 unused1, unused2;
    u64 segsz;
    i64 atime, dtime, ctime;
    int cpid, lpid;
    u64 nattch;
    u64 unused4, unused5;
};
/* struct shminfo64 (asm-generic/shmbuf.h). */
struct k_shminfo {
    u64 shmmax, shmmin, shmmni, shmseg, shmall;
    u64 unused[4];
};
/* struct shm_info (linux/shm.h). */
struct k_shm_info {
    int used_ids;
    int pad;
    u64 shm_tot, shm_rss, shm_swp, swap_attempts, swap_successes;
};
struct k_rlimit {
    u64 cur, max;
};

static __attribute__((unused)) i64 shmget(i64 key, u64 size, i64 flg) { return syscall3(SYS_shmget, key, size, flg); }
static __attribute__((unused)) i64 shmctl(i64 id, i64 cmd, void *buf) { return syscall3(SYS_shmctl, id, cmd, buf); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return shmctl(id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 shm_rm(i64 id) { return shmctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shmat(i64 id, i64 flg) { return syscall3(SYS_shmat, id, 0, flg); }
static __attribute__((unused)) i64 shmdt(i64 addr) { return syscall1(SYS_shmdt, addr); }
static __attribute__((unused)) i64 geteuid(void) { return syscall0(SYS_geteuid); }
static __attribute__((unused)) i64 getegid(void) { return syscall0(SYS_getegid); }
/* Wall clock seconds, the clock the segment times use. */
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x51000000 | (tag << 22) | (int)getpid()); }
/* Run as an unprivileged user: root switches to nobody (65534), so the
 * test sees the same permission checks whoever runs it. */
static __attribute__((unused)) int drop_root(void) {
    if (geteuid() != 0) return 1;
    if (syscall2(SYS_setgroups, 0, 0) != 0) return 0;
    if (syscall3(SYS_setresgid, 65534, 65534, 65534) != 0) return 0;
    return syscall3(SYS_setresuid, 65534, 65534, 65534) == 0;
}
int test_main(void) {
    struct k_shmid_ds ds, st;
    int key = own_key(4);
    i64 id = shmget(key, 777, 0640 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    i64 idx = id & 0x7fff;
    check("id_is_seq_times_32768_plus_index", shm_stat(id, &st) == 0 && id == (i64)st.seq * 32768 + idx, id);

    memset(&ds, 0, sizeof ds);
    i64 r = shmctl(idx, SHM_STAT, &ds);
    check("shm_stat_returns_id", r == id, r);
    check("same_as_ipc_stat", mem_eq(&ds, &st, sizeof ds), 0);
    check("key", ds.key == key, ds.key);
    check("segsz", ds.segsz == 777, (i64)ds.segsz);

    memset(&ds, 0, sizeof ds);
    r = shmctl(idx, SHM_STAT_ANY, &ds);
    check("shm_stat_any_returns_id", r == id, r);
    check("any_same_as_ipc_stat", mem_eq(&ds, &st, sizeof ds), 0);

    r = shmctl(id, SHM_STAT, &ds);
    check("full_id_as_index_ok", r == id, r);
    r = shmctl(idx + 5 * 32768, SHM_STAT, &ds);
    check("sequence_bits_ignored", r == id, r);

    i64 a = shmat(id, 0);
    shm_stat(id, &st);
    shmctl(idx, SHM_STAT, &ds);
    check("attach_seen", ds.nattch == 1 && mem_eq(&ds, &st, sizeof ds), (i64)ds.nattch);
    shm_rm(id);
    r = shmctl(idx, SHM_STAT, &ds);
    check("marked_segment_still_in_slot", r == id && ds.key == 0 && (ds.mode & SHM_DEST), r);
    shmdt(a);
    r = shmctl(idx, SHM_STAT, &ds);
    check("removed_slot_einval", r == -EINVAL, r);
    r = shmctl(idx, SHM_STAT_ANY, &ds);
    check("removed_slot_any_einval", r == -EINVAL, r);
    r = shmctl(-1, SHM_STAT, &ds);
    check("negative_index_einval", r == -EINVAL, r);
    return finish();
}
