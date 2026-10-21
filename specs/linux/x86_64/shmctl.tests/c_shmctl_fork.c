/* Attachments across fork and exit: fork adds an attachment without
 * touching atime or lpid; a child's shmat, shmdt and exit (which detaches)
 * set lpid to the child; an IPC_RMID or IPC_SET by the child is seen by the
 * parent at once.
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
int test_main(void) {
    struct k_shmid_ds ds;
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    check("create_ok", id >= 0, id);
    i64 a = shmat(id, 0);
    check("attach_ok", a > 0, a);
    shm_stat(id, &ds);
    i64 atime0 = ds.atime;

    i64 pid = fork();
    if (pid == 0) {
        sleep_ms(200);
        exit_group(0);
    }
    check("fork_ok", pid > 0, pid);
    shm_stat(id, &ds);
    check("fork_adds_attachment", ds.nattch == 2, (i64)ds.nattch);
    check("fork_keeps_lpid", ds.lpid == getpid(), ds.lpid);
    check("fork_keeps_atime", ds.atime == atime0, ds.atime - atime0);
    check("child_exit_ok", wait_exit(pid) == 0, 0);
    shm_stat(id, &ds);
    check("exit_detaches", ds.nattch == 1, (i64)ds.nattch);
    check("exit_sets_lpid", ds.lpid == pid, ds.lpid == getpid());
    check("exit_sets_dtime", ds.dtime >= atime0, ds.dtime - atime0);
    shmdt(a);
    shm_stat(id, &ds);
    check("parent_detach_lpid", ds.lpid == getpid() && ds.nattch == 0, (i64)ds.nattch);

    /* A child that attaches, changes the mode and removes the segment. */
    volatile unsigned *gate = shared_page(4096);
    pid = fork();
    if (pid == 0) {
        i64 m = shmat(id, 0);
        struct k_shmid_ds c;
        shm_stat(id, &c);
        c.mode = 0640;
        int ok = m > 0 && shmctl(id, IPC_SET, &c) == 0 && shm_rm(id) == 0;
        gate[1] = ok;
        gate_open(gate);
        gate_wait(gate + 2);
        exit_group(0);
    }
    gate_wait(gate);
    check("child_ops_ok", gate[1] == 1, gate[1]);
    i64 r = shm_stat(id, &ds);
    check("parent_sees_child", r == 0 && ds.lpid == pid && ds.nattch == 1, r);
    check("parent_sees_set_and_rmid", ds.mode == (SHM_DEST | 0640), ds.mode);
    gate_open(gate + 2);
    wait_exit(pid);
    r = shm_stat(id, &ds);
    check("gone_when_child_exits", r == -EINVAL, r);
    return finish();
}
