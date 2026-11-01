/* IPC_STAT fills a struct shmid64_ds; IPC_SET takes uid, gid and the low
 * 9 bits of mode from one (other fields ignored: cuid, segsz, times,
 * pids and nattch kept) and sets shm_ctime. uid or gid -1 is EINVAL and
 * changes nothing. The owner may give the segment to any uid and, as
 * creator, keeps owner rights. NULL buffers are EFAULT. IPC_STAT does
 * not change atime/dtime/ctime.
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

int test_main(void) {
    unsigned uid = (unsigned)syscall0(SYS_geteuid), gid = (unsigned)syscall0(SYS_getegid);
    i64 id = shmget(IPC_PRIVATE, 5000, 0600);
    check("shmget_ok", id >= 0, id);
    i64 a = shm_attach(id, 0);
    check("attach_ok", a > 0, a);
    struct k_shmid_ds ds, set;
    i64 r = shm_stat(id, &ds);
    check("stat_ok", r == 0, r);
    check("segsz", ds.segsz == 5000, (i64)ds.segsz);
    check("nattch_1", ds.nattch == 1, (i64)ds.nattch);
    check("cpid_lpid_me", ds.cpid == getpid() && ds.lpid == getpid(), 0);
    i64 atime = ds.atime;

    set = ds;
    set.mode = 0177640;
    set.cuid = 4242;
    set.segsz = 1;
    set.nattch = 9;
    set.cpid = 1;
    set.atime = 1;
    set.dtime = 1;
    set.ctime = 1;
    r = shmctl(id, IPC_SET, &set);
    check("set_ok", r == 0, r);
    shm_stat(id, &ds);
    check("mode_low_9_bits", ds.mode == 0640, ds.mode);
    check("cuid_kept", ds.cuid == uid, ds.cuid);
    check("segsz_kept", ds.segsz == 5000, (i64)ds.segsz);
    check("nattch_kept", ds.nattch == 1, (i64)ds.nattch);
    check("cpid_kept", ds.cpid == getpid(), 0);
    check("atime_kept", ds.atime == atime, ds.atime - atime);
    check("dtime_0", ds.dtime == 0, ds.dtime);
    check("ctime_updated", ds.ctime > 1, ds.ctime);

    set = ds;
    set.uid = 0xffffffffu;
    set.mode = 0600;
    r = shmctl(id, IPC_SET, &set);
    check("uid_minus1_einval", r == -EINVAL, r);
    set.uid = uid;
    set.gid = 0xffffffffu;
    r = shmctl(id, IPC_SET, &set);
    check("gid_minus1_einval", r == -EINVAL, r);
    shm_stat(id, &ds);
    check("einval_changed_nothing", ds.mode == 0640 && ds.gid == gid, ds.mode);

    set.gid = gid;
    set.uid = 777;
    r = shmctl(id, IPC_SET, &set);
    check("give_away_ok", r == 0, r);
    shm_stat(id, &ds);
    check("given_creator_still_stats", ds.uid == 777 && ds.cuid == uid && ds.mode == 0600, ds.uid);
    set.uid = uid;
    check("creator_sets_back", shmctl(id, IPC_SET, &set) == 0, 0);

    r = shmctl(id, IPC_SET, 0);
    check("set_null_efault", r == -EFAULT, r);
    r = shmctl(id, IPC_STAT, 0);
    check("stat_null_efault", r == -EFAULT, r);
    shmdt((u64)a);
    r = shmctl(id, IPC_RMID, (void *)1);
    check("rmid_ignores_buffer", r == 0, r);
    r = shm_stat(id, &ds);
    check("removed_einval", r == -EINVAL, r);
    return finish();
}
