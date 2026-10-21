/* Who may do what, for an unprivileged caller (root switches to nobody
 * first). IPC_STAT and SHM_STAT need read permission in the caller's class
 * (the owner class for the owner or the creator); write or other-class
 * bits do not help. SHM_STAT_ANY needs none. IPC_SET, IPC_RMID, SHM_LOCK
 * and SHM_UNLOCK need no mode bits for the owner. shmat checks the mode.
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
static i64 set_mode(i64 id, unsigned mode) {
    struct k_shmid_ds ds;
    memset(&ds, 0, sizeof ds);
    ds.uid = (unsigned)geteuid();
    ds.gid = (unsigned)getegid();
    ds.mode = mode;
    return shmctl(id, IPC_SET, &ds);
}

int test_main(void) {
    check("unprivileged", drop_root(), 0);
    struct k_shmid_ds ds;
    i64 id = shmget(IPC_PRIVATE, 4096, 0);
    check("create_mode_0_ok", id >= 0, id);
    i64 r = shm_stat(id, &ds);
    check("stat_mode_0_eacces", r == -EACCES, r);
    r = shmctl(id & 0x7fff, SHM_STAT, &ds);
    check("shm_stat_mode_0_eacces", r == -EACCES, r);
    memset(&ds, 0, sizeof ds);
    r = shmctl(id & 0x7fff, SHM_STAT_ANY, &ds);
    check("shm_stat_any_mode_0_ok", r == id, r);
    check("shm_stat_any_fills", ds.mode == 0 && ds.segsz == 4096 && ds.cuid == (unsigned)geteuid(), ds.mode);
    r = shmat(id, 0);
    check("shmat_mode_0_eacces", r == -EACCES, r);
    r = shmctl(id, SHM_LOCK, 0);
    check("lock_mode_0_ok", r == 0, r);
    r = shmctl(id, SHM_UNLOCK, 0);
    check("unlock_mode_0_ok", r == 0, r);

    check("set_0400_ok", set_mode(id, 0400) == 0, 0);
    r = shm_stat(id, &ds);
    check("stat_0400_ok", r == 0 && ds.mode == 0400, r);
    check("set_0200_ok", set_mode(id, 0200) == 0, 0);
    r = shm_stat(id, &ds);
    check("stat_0200_eacces", r == -EACCES, r);
    check("set_0100_ok", set_mode(id, 0100) == 0, 0);
    r = shm_stat(id, &ds);
    check("stat_0100_eacces", r == -EACCES, r);
    check("set_0044_ok", set_mode(id, 0044) == 0, 0);
    r = shm_stat(id, &ds);
    check("stat_group_other_read_eacces", r == -EACCES, r);
    check("set_0004_ok", set_mode(id, 0004) == 0, 0);
    r = shm_stat(id, &ds);
    check("stat_0004_eacces", r == -EACCES, r);
    check("set_0444_ok", set_mode(id, 0444) == 0, 0);
    r = shmctl(id & 0x7fff, SHM_STAT, &ds);
    check("shm_stat_0444_ok", r == id, r);

    check("set_0_ok", set_mode(id, 0) == 0, 0);
    r = shm_rm(id);
    check("rmid_mode_0_ok", r == 0, r);
    r = shm_stat(id, &ds);
    check("gone", r == -EINVAL, r);
    return finish();
}
