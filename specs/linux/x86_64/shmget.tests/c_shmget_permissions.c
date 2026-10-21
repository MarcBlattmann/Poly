/* Finding an existing segment checks the access asked for in the low 9
 * bits of shmflg: the r, w and x bits of all three classes are merged
 * and each merged bit must be granted by the segment's mode for the
 * caller's class (owner here). Missing: EACCES. Asking for nothing (0)
 * always works. A caller with CAP_IPC_OWNER (root) passes every check.
 * Creating a segment asks for no access, so mode 0 is fine.
 * poly-diff: syscalls=names (segment ids depend on the IPC namespace's history)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
#define SYS_shmdt 67
/* linux/ipc.h, linux/shm.h */
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_STAT 2
#define SHM_RDONLY 010000
#define SHM_NORESERVE 010000
#define SHM_DEST 01000

/* struct shmid64_ds (asm-generic/shmbuf.h, struct ipc64_perm from
 * asm-generic/ipcbuf.h) on x86-64. */
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
static __attribute__((unused)) i64 shm_rm(i64 id) { return syscall3(SYS_shmctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_shmctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) i64 shm_attach(i64 id, i64 flg) { return syscall3(SYS_shmat, id, 0, flg); }
/* A key of this test's own, from the pid, so runs do not meet. */
static __attribute__((unused)) int own_key(int tag) { return (int)(0x50000000 | (tag << 22) | (int)getpid()); }
int test_main(void) {
    int root = syscall0(SYS_geteuid) == 0;
    i64 denied = root ? 0 : -EACCES;
    int key = own_key(1);
    i64 id = shmget(key, 100, 0640 | IPC_CREAT | IPC_EXCL);
    check("create_ok", id >= 0, id);

    i64 r = shmget(key, 0, 0);
    check("ask_nothing_ok", r == id, r);
    r = shmget(key, 0, 0600);
    check("owner_rw_ok", r == id, r);
    r = shmget(key, 0, 0400);
    check("owner_r_ok", r == id, r);
    r = shmget(key, 0, 0040);
    check("group_r_bit_ok", r == id, r);
    r = shmget(key, 0, 0020);
    check("group_w_bit_checks_owner_w", r == id, r);
    r = shmget(key, 0, 0006);
    check("other_rw_bits_check_owner_rw", r == id, r);
    r = shmget(key, 0, 0100);
    check("owner_x_denied", r == (root ? id : denied), r);
    r = shmget(key, 0, 0001);
    check("other_x_bit_denied", r == (root ? id : denied), r);
    r = shmget(key, 0, 0777);
    check("all_bits_denied", r == (root ? id : denied), r);
    r = shmget(key, 0, 0700 | IPC_CREAT);
    check("creat_existing_checks_access", r == (root ? id : denied), r);
    shm_rm(id);

    key = own_key(2);
    id = shmget(key, 100, 0400 | IPC_CREAT | IPC_EXCL);
    check("ro_create_ok", id >= 0, id);
    r = shmget(key, 0, 0444);
    check("ro_read_ok", r == id, r);
    r = shmget(key, 0, 0200);
    check("ro_write_denied", r == (root ? id : denied), r);
    r = shmget(key, 0, 0002);
    check("ro_other_w_bit_denied", r == (root ? id : denied), r);
    shm_rm(id);

    key = own_key(3);
    id = shmget(key, 100, IPC_CREAT | IPC_EXCL);
    check("mode0_create_ok", id >= 0, id);
    r = shmget(key, 0, 0);
    check("mode0_ask_nothing_ok", r == id, r);
    r = shmget(key, 0, 0400);
    check("mode0_read_denied", r == (root ? id : denied), r);
    shm_rm(id);
    return finish();
}
