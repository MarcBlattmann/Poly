/* Shared memory without a named file works too: MAP_SHARED|MAP_ANONYMOUS,
 * a MAP_SHARED mapping of /dev/zero opened read-write, and a System V
 * shared memory segment; pgoff counts pages of that memory object.
 * /dev/zero opened read-only and a segment attached with SHM_RDONLY are
 * EINVAL.
 * poly-diff: syscalls=names (System V segment ids depend on the IPC namespace's history)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_remap_file_pages 216
#define SYS_shmget 29
#define SYS_shmat 30
#define SYS_shmctl 31
/* asm-generic/mman-common.h */
#define MAP_NONBLOCK 0x10000
/* linux/magic.h */
#define TMPFS_MAGIC 0x01021994

static A_UNUSED i64 rfp(void *addr, u64 size, u64 prot, u64 pgoff, u64 flags) {
    return syscall5(SYS_remap_file_pages, addr, size, prot, pgoff, flags);
}
static A_UNUSED char *map(void *addr, u64 len, i64 prot, i64 flags, i64 fd, u64 off) {
    return (char *)syscall6(SYS_mmap, (i64)addr, len, prot, flags, fd, off);
}
static A_UNUSED int mapped(char *p) { return (u64)p < (u64)-4095; }

/* A file on the tmpfs at /dev/shm, removed again at once, with `pages`
 * pages: page i is filled with the letter 'A' + i. Returns the fd. */
static A_UNUSED i64 lettered_file(const char *path, int pages) {
    i64 fd = a_open(path, O_RDWR | O_CREAT | O_TRUNC, 0600);
    a_unlink(path);
    if (fd < 0) return fd;
    char pg[A_PAGE];
    for (int i = 0; i < pages; i++) {
        memset(pg, 'A' + i, A_PAGE);
        if (a_write(fd, pg, A_PAGE) != A_PAGE) return -EIO;
    }
    return fd;
}

/* The first byte of each of `n` pages at p, as letters, e.g. "DBCD". */
static A_UNUSED int pages_are(const char *p, const char *want) {
    for (int i = 0; want[i]; i++)
        if (p[(u64)i * A_PAGE] != want[i] || p[(u64)i * A_PAGE + A_PAGE - 1] != want[i]) return 0;
    return 1;
}

/* Read one byte at p in a forked child: 0 if it could, else the signal
 * that killed it (SIGSEGV 11, SIGBUS 7). */
static A_UNUSED i64 probe_read(volatile char *p) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        (void)*p;
        exit_group(0);
    }
    i32 st = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &st, 0, 0) != pid) return -1;
    return A_WIFSIGNALED(st) ? A_WTERMSIG(st) : A_WEXITSTATUS(st) ? 100 + A_WEXITSTATUS(st) : 0;
}
static A_UNUSED i64 probe_write(volatile char *p) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        *p = 'w';
        exit_group(0);
    }
    i32 st = -1;
    if (pid < 0 || syscall4(SYS_wait4, pid, &st, 0, 0) != pid) return -1;
    return A_WIFSIGNALED(st) ? A_WTERMSIG(st) : A_WEXITSTATUS(st) ? 100 + A_WEXITSTATUS(st) : 0;
}
int test_main(void) {
    char *sa = map(0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1, 0);
    check("setup_anon", mapped(sa), 0);
    if (!mapped(sa)) return finish();
    for (int i = 0; i < 4; i++) memset(sa + i * A_PAGE, 'a' + i, A_PAGE);
    i64 r = rfp(sa, 2 * A_PAGE, 0, 2, 0);
    check("anon_ok", r == 0, r);
    check("anon_applied", pages_are(sa, "cdcd"), sa[0]);
    volatile char *v = sa;
    v[5] = 'X';
    check("anon_alias", v[2 * A_PAGE + 5] == 'X', v[2 * A_PAGE + 5]);

    i64 z = a_open("/dev/zero", O_RDWR, 0);
    char *zz = map(0, 3 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, z, 0);
    check("setup_zero", z >= 0 && mapped(zz), z);
    if (mapped(zz)) {
        zz[2 * A_PAGE] = 'k';
        r = rfp(zz, A_PAGE, 0, 2, 0);
        check("zero_ok", r == 0, r);
        check("zero_applied", zz[0] == 'k' && zz[A_PAGE] == 0, zz[0]);
    }
    i64 zr = a_open("/dev/zero", O_RDONLY, 0);
    char *zro = map(0, 2 * A_PAGE, PROT_READ, MAP_SHARED, zr, 0);
    check("setup_zero_rdonly", zr >= 0 && mapped(zro), zr);
    r = rfp(zro, A_PAGE, 0, 1, 0);
    check("zero_rdonly_einval", r == -EINVAL, r);

    i64 id = syscall3(SYS_shmget, 0 /* IPC_PRIVATE */, 4 * A_PAGE, 01000 /* IPC_CREAT */ | 0600);
    check("setup_shmget", id >= 0, id);
    if (id >= 0) {
        char *sh = (char *)syscall3(SYS_shmat, id, 0, 0);
        char *shr = (char *)syscall3(SYS_shmat, id, 0, 010000 /* SHM_RDONLY */);
        syscall3(SYS_shmctl, id, 0 /* IPC_RMID */, 0);
        check("setup_shmat", mapped(sh) && mapped(shr), 0);
        if (mapped(sh) && mapped(shr)) {
            for (int i = 0; i < 4; i++) sh[i * A_PAGE] = (char)('p' + i);
            r = rfp(sh + A_PAGE, A_PAGE, 0, 3, 0);
            check("shm_ok", r == 0, r);
            check("shm_applied", sh[0] == 'p' && sh[A_PAGE] == 's' && sh[2 * A_PAGE] == 'r', sh[A_PAGE]);
            r = rfp(shr, A_PAGE, 0, 3, 0);
            check("shm_rdonly_einval", r == -EINVAL, r);
            check("shm_rdonly_kept", shr[0] == 'p', shr[0]);
        }
    }
    return finish();
}
