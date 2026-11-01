/* A segment marked with IPC_RMID lives until its last attachment goes:
 * after the first of two shmdt calls it still exists (IPC_STAT works,
 * nattch 1, SHM_DEST set) and the other attachment still shows the
 * data; the last shmdt destroys it (the id is EINVAL).
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
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    check("shmget_ok", id >= 0, id);
    char *a = (char *)shm_attach(id, 0);
    char *b = (char *)shm_attach(id, SHM_RDONLY);
    check("two_attachments", (i64)a > 0 && (i64)b > 0 && a != b, 0);
    a[10] = 'q';
    check("shared", b[10] == 'q', b[10]);
    check("rm_ok", shm_rm(id) == 0, 0);
    i64 r = shmdt((u64)a);
    check("first_detach_ok", r == 0, r);
    struct k_shmid_ds ds;
    r = shm_stat(id, &ds);
    check("still_exists", r == 0, r);
    check("nattch_1", ds.nattch == 1, (i64)ds.nattch);
    check("dest_set", (ds.mode & SHM_DEST) != 0, ds.mode);
    check("data_still_there", b[10] == 'q', b[10]);
    r = shmdt((u64)b);
    check("last_detach_ok", r == 0, r);
    r = shm_stat(id, &ds);
    check("destroyed", r == -EINVAL, r);
    r = syscall3(SYS_shmat, id, 0, 0);
    check("attach_destroyed_einval", r == -EINVAL, r);
    return finish();
}
