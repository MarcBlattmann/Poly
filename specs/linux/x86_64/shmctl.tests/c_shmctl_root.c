/* Privilege: root reads and controls any segment; another user without
 * privilege may IPC_STAT only with read permission in its class and gets
 * EPERM for IPC_SET, IPC_RMID, SHM_LOCK and SHM_UNLOCK unless it is the
 * owner or the creator. Giving the segment to that user lets it control it.
 * poly-diff: needs=root (switches a child to another user)
 * poly-diff: syscalls=off (two processes)
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
static volatile unsigned *page;

/* The checks a child running as nobody (65534) makes; results in page[1..]. */
static void as_nobody(i64 id, int step) {
    i64 pid = fork();
    if (pid == 0) {
        int ok = syscall2(SYS_setgroups, 0, 0) == 0 && syscall3(SYS_setresgid, 65534, 65534, 65534) == 0 &&
                 syscall3(SYS_setresuid, 65534, 65534, 65534) == 0;
        struct k_shmid_ds ds;
        memset(&ds, 0, sizeof ds);
        page[0] = ok;
        page[1] = (unsigned)-shm_stat(id, &ds);
        if (step == 1) {
            ds.uid = 65534;
            ds.gid = 65534;
            ds.mode = 0666;
            page[2] = (unsigned)-shmctl(id, IPC_SET, &ds);
            page[3] = (unsigned)-shm_rm(id);
            page[4] = (unsigned)-shmctl(id, SHM_LOCK, 0);
            page[5] = (unsigned)-shmctl(id, SHM_UNLOCK, 0);
            page[6] = (unsigned)-shmctl(id, IPC_SET, 0);
            page[7] = (unsigned)shmctl(id & 0x7fff, SHM_STAT_ANY, &ds) == (unsigned)id;
        } else {
            ds.mode = 0600;
            page[2] = (unsigned)-shmctl(id, IPC_SET, &ds);
            page[3] = (unsigned)-shm_rm(id);
        }
        exit_group(0);
    }
    wait_exit(pid);
}

int test_main(void) {
    check("is_root", geteuid() == 0, geteuid());
    page = shared_page(4096);
    struct k_shmid_ds ds;
    i64 id = shmget(IPC_PRIVATE, 4096, 0);
    check("create_ok", id >= 0, id);
    i64 r = shm_stat(id, &ds);
    check("root_stats_mode_0", r == 0, r);
    r = shmctl(id & 0x7fff, SHM_STAT, &ds);
    check("root_shm_stat_mode_0", r == id, r);
    i64 a = shmat(id, 0);
    check("root_attaches_mode_0", a > 0, a);
    shmdt(a);

    as_nobody(id, 1);
    check("child_is_nobody", page[0] == 1, page[0]);
    check("other_stat_mode_0_eacces", page[1] == EACCES, page[1]);
    check("other_set_eperm", page[2] == EPERM, page[2]);
    check("other_rmid_eperm", page[3] == EPERM, page[3]);
    check("other_lock_eperm", page[4] == EPERM, page[4]);
    check("other_unlock_eperm", page[5] == EPERM, page[5]);
    check("other_set_bad_buf_efault", page[6] == EFAULT, page[6]);
    check("other_shm_stat_any_ok", page[7] == 1, page[7]);
    r = shm_stat(id, &ds);
    check("nothing_changed", r == 0 && ds.mode == 0 && ds.uid == 0, ds.mode);

    ds.mode = 0604;
    shmctl(id, IPC_SET, &ds);
    as_nobody(id, 1);
    check("other_stat_0604_ok", page[1] == 0, page[1]);
    check("other_set_still_eperm", page[2] == EPERM, page[2]);
    ds.mode = 0640;
    shmctl(id, IPC_SET, &ds);
    as_nobody(id, 1);
    check("other_stat_group_bits_eacces", page[1] == EACCES, page[1]);

    /* Root gives the segment to nobody, who then owns it. */
    ds.uid = 65534;
    ds.gid = 65534;
    ds.mode = 0;
    r = shmctl(id, IPC_SET, &ds);
    check("root_gives_away", r == 0, r);
    r = shm_stat(id, &ds);
    check("root_still_stats", r == 0 && ds.uid == 65534 && ds.cuid == 0, r);
    as_nobody(id, 2);
    check("owner_stat_mode_0_eacces", page[1] == EACCES, page[1]);
    check("owner_set_ok", page[2] == 0, page[2]);
    check("owner_rmid_ok", page[3] == 0, page[3]);
    r = shm_stat(id, &ds);
    check("removed_by_owner", r == -EINVAL, r);
    return finish();
}
