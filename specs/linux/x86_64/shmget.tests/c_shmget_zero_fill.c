/* The memory of a new segment reads as zeros, through all its pages: a
 * segment of 4097 bytes covers two whole pages once attached. Two
 * attachments of one id see the same bytes. A segment made again with a
 * key that was removed starts zeroed, not with the old contents.
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
    i64 id = shmget(IPC_PRIVATE, 4097, 0600);
    check("create_ok", id >= 0, id);
    volatile unsigned char *a = (volatile unsigned char *)shm_attach(id, 0);
    volatile unsigned char *b = (volatile unsigned char *)shm_attach(id, 0);
    check("attach_twice", (i64)a > 0 && (i64)b > 0 && a != b, 0);
    int zero = 1;
    for (int i = 0; i < 8192; i++)
        if (a[i]) zero = 0;
    check("zero_filled_two_pages", zero, 0);
    a[0] = 1;
    a[4096] = 2;
    a[8191] = 3;
    check("shared_between_attachments", b[0] == 1 && b[4096] == 2 && b[8191] == 3, 0);
    syscall1(SYS_shmdt, a);
    syscall1(SYS_shmdt, b);
    shm_rm(id);

    int key = own_key(1);
    id = shmget(key, 4096, 0600 | IPC_CREAT | IPC_EXCL);
    a = (volatile unsigned char *)shm_attach(id, 0);
    for (int i = 0; i < 4096; i++) a[i] = 0xa5;
    syscall1(SYS_shmdt, a);
    shm_rm(id);
    id = shmget(key, 4096, 0600 | IPC_CREAT | IPC_EXCL);
    check("recreate_ok", id >= 0, id);
    a = (volatile unsigned char *)shm_attach(id, 0);
    zero = 1;
    for (int i = 0; i < 4096; i++)
        if (a[i]) zero = 0;
    check("recreated_zeroed", zero, 0);
    syscall1(SYS_shmdt, a);
    shm_rm(id);
    return finish();
}
