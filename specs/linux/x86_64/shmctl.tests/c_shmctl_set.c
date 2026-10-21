/* IPC_SET: takes uid, gid and the low 9 bits of mode from the buffer and
 * ignores every other field; updates ctime. The owner may give the
 * segment away (any valid uid/gid); the creator keeps control. A uid or
 * gid of -1 is EINVAL.
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
    check("unprivileged", drop_root(), 0);
    unsigned me = (unsigned)geteuid(), mygid = (unsigned)getegid();
    int key = own_key(2);
    i64 id = shmget(key, 3000, 0600 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);
    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    i64 ctime0 = ds.ctime;
    sleep_ms(1100);

    /* Only uid, gid and mode & 0777 are taken. */
    ds.mode = 0177664;
    ds.key = 99;
    ds.cuid = 4242;
    ds.cgid = 4343;
    ds.segsz = 1;
    ds.atime = ds.dtime = ds.ctime = 12345;
    ds.cpid = ds.lpid = 777;
    ds.nattch = 9;
    i64 r = shmctl(id, IPC_SET, &ds);
    check("set_returns_0", r == 0, r);
    shm_stat(id, &ds);
    check("mode_low_9_bits", ds.mode == 0664, ds.mode);
    check("key_kept", ds.key == key, ds.key);
    check("cuid_kept", ds.cuid == me, ds.cuid);
    check("cgid_kept", ds.cgid == mygid, ds.cgid);
    check("segsz_kept", ds.segsz == 3000, (i64)ds.segsz);
    check("atime_kept", ds.atime == 0, ds.atime);
    check("cpid_kept", ds.cpid == getpid(), ds.cpid);
    check("lpid_kept", ds.lpid == 0, ds.lpid);
    check("nattch_kept", ds.nattch == 0, (i64)ds.nattch);
    check("ctime_advanced", ds.ctime > ctime0, ds.ctime - ctime0);
    check("ctime_now", ds.ctime <= now_sec() && ds.ctime >= now_sec() - 1, 0);

    /* Invalid ids: nothing changes. */
    ds.uid = 0xffffffffu;
    r = shmctl(id, IPC_SET, &ds);
    check("uid_minus_1_einval", r == -EINVAL, r);
    ds.uid = me;
    ds.gid = 0xffffffffu;
    r = shmctl(id, IPC_SET, &ds);
    check("gid_minus_1_einval", r == -EINVAL, r);
    shm_stat(id, &ds);
    check("unchanged_after_einval", ds.uid == me && ds.gid == mygid && ds.mode == 0664, ds.uid);

    /* Give it away; the creator still controls it and is checked as the owner. */
    ds.uid = 12345;
    ds.gid = 54321;
    ds.mode = 0600;
    r = shmctl(id, IPC_SET, &ds);
    check("give_away_ok", r == 0, r);
    shm_stat(id, &ds);
    check("new_uid", ds.uid == 12345, ds.uid);
    check("new_gid", ds.gid == 54321, ds.gid);
    check("cuid_still_me", ds.cuid == me, ds.cuid);
    ds.mode = 0066;
    r = shmctl(id, IPC_SET, &ds);
    check("creator_may_set", r == 0, r);
    r = shm_stat(id, &ds);
    check("creator_checked_as_owner", r == -EACCES, r);
    ds.uid = me;
    ds.gid = mygid;
    ds.mode = 0400;
    r = shmctl(id, IPC_SET, &ds);
    check("creator_takes_back", r == 0, r);
    r = shm_stat(id, &ds);
    check("stat_after_take_back", r == 0 && ds.uid == me && ds.mode == 0400, r);

    r = shm_rm(id);
    check("creator_may_rmid", r == 0, r);
    return finish();
}
