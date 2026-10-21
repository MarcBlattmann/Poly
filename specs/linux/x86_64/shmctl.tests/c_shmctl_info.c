/* IPC_INFO and SHM_INFO: both return the highest slot index in use (at
 * least the slot of a live segment, id & 0x7fff); IPC_INFO fills struct
 * shminfo64 with the default limits; SHM_INFO counts segments, their
 * pages (rounded up per segment) and resident pages. The shmid argument
 * is ignored (any non-negative value).
 * poly-diff: syscalls=names (return values depend on the IPC namespace's history)
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
    struct k_shminfo si;
    struct k_shm_info before, after;
    memset(&before, 0x55, sizeof before);
    i64 r0 = shmctl(0, SHM_INFO, &before);
    check("shm_info_ok", r0 >= 0, r0);
    check("shm_info_pad_zero", before.pad == 0, before.pad);
    check("shm_info_swap_zero", before.shm_swp == 0 && before.swap_attempts == 0 && before.swap_successes == 0,
          (i64)before.shm_swp);

    i64 a = shmget(IPC_PRIVATE, 5000, 0600);
    i64 b = shmget(IPC_PRIVATE, 1, 0600);
    check("create_ok", a >= 0 && b >= 0, a);
    i64 top = (a & 0x7fff) > (b & 0x7fff) ? (a & 0x7fff) : (b & 0x7fff);

    memset(&si, 0x55, sizeof si);
    i64 r = shmctl(12345, IPC_INFO, &si);
    check("ipc_info_returns_highest_index", r >= top && r < 32768, r - top);
    check("shmmax_default", si.shmmax == 0xfffffffffeffffffUL, (i64)si.shmmax);
    check("shmmin_1", si.shmmin == 1, (i64)si.shmmin);
    check("shmmni_4096", si.shmmni == 4096, (i64)si.shmmni);
    check("shmseg_4096", si.shmseg == 4096, (i64)si.shmseg);
    check("shmall_default", si.shmall == 0xfffffffffeffffffUL, (i64)si.shmall);
    check("unused_zero", si.unused[0] == 0 && si.unused[1] == 0 && si.unused[2] == 0 && si.unused[3] == 0,
          (i64)si.unused[0]);

    i64 r2 = shmctl(77, SHM_INFO, &after);
    check("shm_info_same_index", r2 == r, r2 - r);
    check("used_ids_plus_2", after.used_ids == before.used_ids + 2, after.used_ids - before.used_ids);
    check("tot_plus_3_pages", after.shm_tot == before.shm_tot + 3, (i64)(after.shm_tot - before.shm_tot));
    check("rss_unchanged_untouched", after.shm_rss == before.shm_rss, (i64)(after.shm_rss - before.shm_rss));

    i64 m = shmat(a, 0);
    check("attach_ok", m > 0, m);
    ((volatile char *)m)[0] = 1;
    ((volatile char *)m)[4999] = 1;
    shmctl(0, SHM_INFO, &after);
    check("rss_plus_2_touched", after.shm_rss == before.shm_rss + 2, (i64)(after.shm_rss - before.shm_rss));
    shmdt(m);
    shmctl(0, SHM_INFO, &after);
    check("rss_kept_after_detach", after.shm_rss == before.shm_rss + 2, (i64)(after.shm_rss - before.shm_rss));

    shm_rm(a);
    shm_rm(b);
    shmctl(0, SHM_INFO, &after);
    check("used_ids_back", after.used_ids == before.used_ids, after.used_ids - before.used_ids);
    check("tot_back", after.shm_tot == before.shm_tot, (i64)(after.shm_tot - before.shm_tot));
    check("rss_back", after.shm_rss == before.shm_rss, (i64)(after.shm_rss - before.shm_rss));
    return finish();
}
