/* SHM_EXEC maps the segment executable: code written through a
 * read-write attachment runs from a SHM_EXEC|SHM_RDONLY attachment of
 * the same segment. Without SHM_EXEC the attachment is not executable:
 * jumping into it kills the caller with SIGSEGV.
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
    i64 id = shmget(IPC_PRIVATE, 4096, 0700);
    i64 rw = shmat(id, 0, 0);
    check("rw_attach_ok", rw > 0, rw);
    /* mov $42, %eax; ret */
    static const unsigned char code[] = { 0xb8, 42, 0, 0, 0, 0xc3 };
    for (unsigned i = 0; i < sizeof code; i++) ((volatile unsigned char *)rw)[i] = code[i];

    i64 x = shmat(id, 0, SHM_EXEC | SHM_RDONLY);
    check("exec_attach_ok", x > 0, x);
    int (*fn)(void) = (int (*)(void))x;
    int got = fn();
    check("runs_from_segment", got == 42, got);

    i64 xw = shmat(id, 0, SHM_EXEC);
    check("exec_rw_attach_ok", xw > 0, xw);
    ((volatile unsigned char *)rw)[1] = 7;
    got = ((int (*)(void))xw)();
    check("sees_new_code", got == 7, got);

    i64 pid = fork();
    if (pid == 0) {
        ((int (*)(void))rw)();
        exit_group(0);
    }
    int s = wait_status(pid);
    check("no_exec_segv", W_SIGNALED(s) && W_TERMSIG(s) == SIGSEGV, s);
    shmdt(rw);
    shmdt(x);
    shmdt(xw);
    shm_rm(id);
    return finish();
}
