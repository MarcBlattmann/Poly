/* Segments are system-wide, not per process: a child makes a keyed
 * segment, writes into it and exits; the segment outlives it (exit only
 * detaches), the parent finds it by key with the child's data, and
 * IPC_STAT names the child as creator (cpid) and last attacher (lpid),
 * with no attachments left. A segment made before fork is found by key
 * in the child too.
 * poly-diff: syscalls=off (two processes)
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
    int key = own_key(6);
    i64 pid = fork();
    if (pid == 0) {
        i64 id = shmget(key, 64, 0600 | IPC_CREAT | IPC_EXCL);
        if (id < 0) exit_group(1);
        i64 p = shm_attach(id, 0);
        if (p <= 0) exit_group(2);
        ((volatile char *)p)[3] = 42;
        exit_group(0);
    }
    check("child_ok", wait_exit(pid) == 0, 0);
    i64 id = shmget(key, 0, 0600);
    check("segment_outlives_child", id >= 0, id);
    struct k_shmid_ds ds;
    shm_stat(id, &ds);
    check("cpid_is_child", ds.cpid == pid, 0);
    check("lpid_is_child", ds.lpid == pid, 0);
    check("exit_detached", ds.nattch == 0, (i64)ds.nattch);
    check("size_from_child", ds.segsz == 64, (i64)ds.segsz);
    i64 p = shm_attach(id, 0);
    check("child_data_visible", p > 0 && ((volatile char *)p)[3] == 42, 0);
    syscall1(SYS_shmdt, p);
    shm_rm(id);

    int key2 = own_key(7);
    i64 id2 = shmget(key2, 100, 0600 | IPC_CREAT | IPC_EXCL);
    pid = fork();
    if (pid == 0) exit_group(shmget(key2, 0, 0) == id2 ? 0 : 3);
    check("child_finds_parent_segment", wait_exit(pid) == 0, 0);
    shm_rm(id2);
    return finish();
}
