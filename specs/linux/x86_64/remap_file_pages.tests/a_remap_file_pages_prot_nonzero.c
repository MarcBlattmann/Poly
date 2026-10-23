/* The prot argument must be 0: any other value (PROT_READ, PROT_WRITE,
 * PROT_EXEC, PROT_NONE's neighbours, a bit above the low 32) is EINVAL,
 * and the mapping is left as it was.
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
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_prot_nonzero", 4);
    char *m = map(0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && mapped(m), fd);
    if (fd < 0 || !mapped(m)) return finish();

    i64 r = rfp(m, A_PAGE, PROT_READ, 2, 0);
    check("prot_read_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, PROT_READ | PROT_WRITE, 2, 0);
    check("prot_rw_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, PROT_EXEC, 2, 0);
    check("prot_exec_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, 8, 2, 0);
    check("prot_8_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, 1UL << 32, 2, 0);
    check("prot_bit32_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, ~0UL, 2, 0);
    check("prot_all_einval", r == -EINVAL, r);
    check("mapping_unchanged", pages_are(m, "ABCD"), m[0]);

    /* prot is checked even when nothing else is right. */
    r = rfp(0, 0, PROT_READ, ~0UL, 0);
    check("prot_with_all_bad_einval", r == -EINVAL, r);

    r = rfp(m, A_PAGE, 0, 2, 0);
    check("prot_zero_ok", r == 0, r);
    check("prot_zero_applied", pages_are(m, "CBCD"), m[0]);
    return finish();
}
