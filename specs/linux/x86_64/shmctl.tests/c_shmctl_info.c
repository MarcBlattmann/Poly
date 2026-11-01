/* IPC_INFO fills a struct shminfo64 {shmmax, shmmin, shmmni, shmseg,
 * shmall, 4 unused} (unsigned longs) and returns the highest index in
 * use; shmid is not looked up. Defaults: shmmax and shmall
 * 18446744073692774399, shmmin 1, shmmni 4096, shmseg 4096. SHM_INFO
 * fills a struct shm_info {int used_ids; unsigned long shm_tot (pages
 * of all segments), shm_rss, shm_swp, swap_attempts, swap_successes}
 * and returns the same index. An unwritable buffer is EFAULT. SHM_STAT
 * and SHM_STAT_ANY take an index (the id modulo 32768) and return the
 * segment's id; an index not in use is EINVAL.
 * poly-diff: syscalls=names (segment ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
#define SYS_setrlimit 160
#define SYS_getrlimit 97
/* asm-generic/errno-base.h, asm-generic/errno.h */
#define EIDRM 43
/* linux/ipc.h, linux/shm.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2
#define IPC_INFO 3
#define SHM_LOCK 11
#define SHM_UNLOCK 12
#define SHM_STAT 13
#define SHM_INFO 14
#define SHM_STAT_ANY 15
#define SHM_RDONLY 010000
#define SHM_DEST 01000
#define SHM_LOCKED 02000
/* asm-generic/resource.h */
#define RLIMIT_MEMLOCK 8

/* struct shmid64_ds (asm-generic/shmbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64: 112 bytes. */
struct k_shmid_ds {
    int key;
    unsigned uid, gid, cuid, cgid;
    unsigned short mode, pad1;
    unsigned short seq, pad2;
    unsigned pad3;
    u64 unused1, unused2;
    u64 segsz;
    i64 atime, dtime, ctime;
    int cpid, lpid;
    u64 nattch;
    u64 unused4, unused5;
};

static __attribute__((unused)) i64 shmget(i64 key, u64 size, i64 flg) { return syscall3(SYS_shmget, key, size, flg); }
static __attribute__((unused)) i64 shmctl(i64 id, i64 cmd, void *buf) { return syscall3(SYS_shmctl, id, cmd, buf); }
static __attribute__((unused)) i64 shm_rm(i64 id) { return shmctl(id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return shmctl(id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 shm_attach(i64 id, i64 flg) { return syscall3(SYS_shmat, id, 0, flg); }
static __attribute__((unused)) i64 shmdt(u64 addr) { return syscall1(SYS_shmdt, addr); }
static __attribute__((unused)) i64 now_sec(void) {
    struct k_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &t);
    return t.tv_sec;
}
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x40000000 | (tag << 22) | (int)getpid()); }
/* Whether touching `p` (reading one byte) kills a child with SIGSEGV. */
static __attribute__((unused)) int read_faults(const volatile char *p) {
    i64 pid = fork();
    if (pid == 0) {
        (void)*p;
        exit_group(0);
    }
    int s = wait_status(pid);
    return s != -1 && W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV;
}

struct k_shminfo64 {
    u64 shmmax, shmmin, shmmni, shmseg, shmall, unused[4];
};
struct k_shm_info {
    int used_ids;
    int pad;
    u64 shm_tot, shm_rss, shm_swp, swap_attempts, swap_successes;
};

int test_main(void) {
    i64 id = shmget(IPC_PRIVATE, 3 * 4096 + 1, 0600);
    check("shmget_ok", id >= 0, id);
    struct k_shminfo64 si;
    memset(&si, 0xee, sizeof si);
    i64 r = shmctl(0x7ffffff0, IPC_INFO, &si);
    check("ipc_info_ok", r >= (id & 0x7fff), r);
    check("shmmax", si.shmmax == 18446744073692774399UL, (i64)si.shmmax);
    check("shmmin", si.shmmin == 1, (i64)si.shmmin);
    check("shmmni", si.shmmni == 4096, (i64)si.shmmni);
    check("shmseg", si.shmseg == 4096, (i64)si.shmseg);
    check("shmall", si.shmall == 18446744073692774399UL, (i64)si.shmall);

    struct k_shm_info sh;
    memset(&sh, 0xee, sizeof sh);
    i64 r2 = shmctl(0, SHM_INFO, &sh);
    check("shm_info_same_return", r2 == r, r2);
    check("shm_info_used_ids", sh.used_ids >= 1, sh.used_ids);
    check("shm_info_tot_pages", sh.shm_tot >= 4 && sh.shm_tot < (1UL << 40), (i64)sh.shm_tot);
    r = shmctl(0, IPC_INFO, 0);
    check("ipc_info_null_efault", r == -EFAULT, r);
    r = shmctl(0, SHM_INFO, 0);
    check("shm_info_null_efault", r == -EFAULT, r);

    struct k_shmid_ds ds;
    memset(&ds, 0, sizeof ds);
    r = shmctl(id & 0x7fff, SHM_STAT, &ds);
    check("shm_stat_returns_id", r == id, r);
    check("shm_stat_segsz", ds.segsz == 3 * 4096 + 1, (i64)ds.segsz);
    memset(&ds, 0, sizeof ds);
    r = shmctl(id & 0x7fff, SHM_STAT_ANY, &ds);
    check("shm_stat_any_returns_id", r == id && ds.segsz == 3 * 4096 + 1, r);
    r = shmctl(32767, SHM_STAT, &ds);
    check("shm_stat_unused_einval", r == -EINVAL, r);
    r = shmctl(id & 0x7fff, SHM_STAT, 0);
    check("shm_stat_null_efault", r == -EFAULT, r);
    shm_rm(id);
    r = shmctl(id & 0x7fff, SHM_STAT, &ds);
    check("shm_stat_removed_einval", r == -EINVAL, r);
    return finish();
}
