/* A SHM_RDONLY attachment is mapped read-only for good: a write through
 * it kills the writer with SIGSEGV (the segment keeps its bytes), and
 * mprotect cannot add PROT_WRITE to it (EACCES), though it may lower it
 * to PROT_NONE and back to PROT_READ. A read-write attachment can be
 * mprotected freely, PROT_EXEC included.
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
#define IPC_RMID 0
#define IPC_STAT 2
#define SHM_RDONLY 010000
#define SHM_RND 020000
#define SHM_REMAP 040000
#define SHM_EXEC 0100000
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
static __attribute__((unused)) i64 shmat(i64 id, u64 addr, i64 flg) { return syscall3(SYS_shmat, id, addr, flg); }
static __attribute__((unused)) i64 shmdt(i64 addr) { return syscall1(SYS_shmdt, addr); }
static __attribute__((unused)) i64 shm_rm(i64 id) { return syscall3(SYS_shmctl, id, IPC_RMID, 0); }
static __attribute__((unused)) i64 shm_stat(i64 id, struct k_shmid_ds *ds) {
    memset(ds, 0, sizeof *ds);
    return syscall3(SYS_shmctl, id, IPC_STAT, ds);
}
static __attribute__((unused)) int is_root(void) { return syscall0(SYS_geteuid) == 0; }
/* The start of `len` bytes of address space that are free right now. */
static __attribute__((unused)) u64 free_range(u64 len) {
    i64 r = syscall6(SYS_mmap, 0, len, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall2(SYS_munmap, r, len);
    return (u64)r;
}
int test_main(void) {
    i64 id = shmget(IPC_PRIVATE, 4096, 0600);
    i64 rw = shmat(id, 0, 0);
    i64 ro = shmat(id, 0, SHM_RDONLY);
    check("attach_ok", rw > 0 && ro > 0, ro);
    ((volatile char *)rw)[10] = 1;
    check("ro_reads", ((volatile char *)ro)[10] == 1, ((volatile char *)ro)[10]);

    i64 pid = fork();
    if (pid == 0) {
        ((volatile char *)ro)[10] = 2;
        exit_group(0);
    }
    int s = wait_status(pid);
    check("write_killed_segv", W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV, s);
    check("bytes_kept", ((volatile char *)rw)[10] == 1, ((volatile char *)rw)[10]);

    i64 r = syscall3(SYS_mprotect, ro, 4096, PROT_READ | PROT_WRITE);
    check("mprotect_write_eacces", r == -EACCES, r);
    r = syscall3(SYS_mprotect, ro, 4096, PROT_NONE);
    check("mprotect_none_ok", r == 0, r);
    r = syscall3(SYS_mprotect, ro, 4096, PROT_READ);
    check("mprotect_read_ok", r == 0, r);
    check("ro_reads_again", ((volatile char *)ro)[10] == 1, ((volatile char *)ro)[10]);
    r = syscall3(SYS_mprotect, rw, 4096, PROT_READ);
    check("rw_mprotect_read_ok", r == 0, r);
    r = syscall3(SYS_mprotect, rw, 4096, PROT_READ | PROT_WRITE | 4);
    check("rw_mprotect_rwx_ok", r == 0, r);
    ((volatile char *)rw)[11] = 4;
    check("rw_writes_again", ((volatile char *)ro)[11] == 4, ((volatile char *)ro)[11]);
    shmdt(rw);
    shmdt(ro);
    shm_rm(id);
    return finish();
}
