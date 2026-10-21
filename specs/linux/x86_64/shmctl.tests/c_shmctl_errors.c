/* Argument errors: unknown commands (including IPC_64 or'd in, and
 * negative values) and negative ids are EINVAL for every command; an id
 * whose slot or sequence does not match a live segment is EINVAL; a bad
 * buffer is EFAULT. IPC_STAT looks the id up before touching the buffer,
 * IPC_SET reads the buffer first.
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
    struct k_shmid_ds ds;
    struct k_shminfo si;
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    check("create_ok", id >= 0, id);
    i64 r;
    r = shmctl(id, 4, &ds);
    check("cmd_4_einval", r == -EINVAL, r);
    r = shmctl(id, 10, &ds);
    check("cmd_10_einval", r == -EINVAL, r);
    r = shmctl(id, 16, &ds);
    check("cmd_16_einval", r == -EINVAL, r);
    r = shmctl(id, 99, &ds);
    check("cmd_99_einval", r == -EINVAL, r);
    r = shmctl(id, -1, &ds);
    check("cmd_minus_1_einval", r == -EINVAL, r);
    r = shmctl(id, IPC_STAT | IPC_64, &ds);
    check("ipc_64_flag_einval", r == -EINVAL, r);
    r = shmctl(id, IPC_RMID | IPC_64, 0);
    check("rmid_ipc_64_einval", r == -EINVAL, r);

    r = shmctl(-1, IPC_STAT, &ds);
    check("stat_id_minus_1_einval", r == -EINVAL, r);
    r = shmctl(-1, IPC_RMID, 0);
    check("rmid_id_minus_1_einval", r == -EINVAL, r);
    r = shmctl(-5, IPC_INFO, &si);
    check("info_negative_id_einval", r == -EINVAL, r);
    r = shmctl(-5, SHM_INFO, &si);
    check("shm_info_negative_id_einval", r == -EINVAL, r);
    r = shmctl(0x7fffffff, IPC_STAT, &ds);
    check("stat_unused_id_einval", r == -EINVAL, r);
    r = shmctl(id + 32768, IPC_STAT, &ds);
    check("stat_wrong_sequence_einval", r == -EINVAL, r);

    r = shmctl(id, IPC_STAT, 0);
    check("stat_null_efault", r == -EFAULT, r);
    r = shmctl(id, IPC_STAT, (void *)8);
    check("stat_bad_ptr_efault", r == -EFAULT, r);
    r = shmctl(id, IPC_SET, 0);
    check("set_null_efault", r == -EFAULT, r);
    r = shmctl(0, IPC_INFO, 0);
    check("info_null_efault", r == -EFAULT, r);
    r = shmctl(0, SHM_INFO, 0);
    check("shm_info_null_efault", r == -EFAULT, r);
    r = shmctl(id & 0x7fff, SHM_STAT, 0);
    check("shm_stat_null_efault", r == -EFAULT, r);

    /* Order. */
    r = shmctl(0x7fffffff, IPC_STAT, 0);
    check("stat_bad_id_before_bad_buf", r == -EINVAL, r);
    r = shmctl(0x7fffffff, IPC_SET, 0);
    check("set_bad_buf_before_bad_id", r == -EFAULT, r);
    r = shmctl(-1, IPC_SET, 0);
    check("negative_id_before_bad_buf", r == -EINVAL, r);
    r = shmctl(-1, 99, 0);
    check("negative_id_bad_cmd_einval", r == -EINVAL, r);

    /* Nothing changed. */
    r = shm_stat(id, &ds);
    check("segment_intact", r == 0 && ds.mode == 0600 && ds.segsz == 4096, r);
    check("rmid_ok", shm_rm(id) == 0, 0);
    return finish();
}
