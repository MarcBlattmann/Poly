/* The same file page can be mapped at several places of one mapping:
 * all of them are the same memory, and writes through any of them reach
 * that page of the file (pread sees them), and later writes to the file
 * show through the mapping.
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
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_alias", 4);
    char *m = map(0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && mapped(m), fd);
    if (fd < 0 || !mapped(m)) return finish();

    check("remap_0", rfp(m, A_PAGE, 0, 2, 0) == 0, 0);
    check("remap_3", rfp(m + 3 * A_PAGE, A_PAGE, 0, 2, 0) == 0, 0);
    check("three_times_C", pages_are(m, "CBCC"), m[0]);

    volatile char *v = m;
    v[10] = 'x';
    check("alias_2_sees", v[2 * A_PAGE + 10] == 'x', v[2 * A_PAGE + 10]);
    check("alias_3_sees", v[3 * A_PAGE + 10] == 'x', v[3 * A_PAGE + 10]);
    v[3 * A_PAGE + 11] = 'y';
    check("alias_0_sees", v[11] == 'y', v[11]);
    char buf[2] = { 0, 0 };
    i64 r = syscall4(SYS_pread64, fd, buf, 2, 2 * A_PAGE + 10);
    check("file_page_2_has_writes", r == 2 && buf[0] == 'x' && buf[1] == 'y', r);
    r = syscall4(SYS_pread64, fd, buf, 2, 10);
    check("file_page_0_untouched", r == 2 && buf[0] == 'A' && buf[1] == 'A', buf[0]);

    r = syscall4(SYS_pwrite64, fd, "z", 1, 2 * A_PAGE + 20);
    check("pwrite_ok", r == 1, r);
    check("pwrite_seen_0", m[20] == 'z', m[20]);
    check("pwrite_seen_3", m[3 * A_PAGE + 20] == 'z', m[3 * A_PAGE + 20]);

    /* Page 0 of the file is no longer mapped anywhere in m. */
    r = syscall4(SYS_pwrite64, fd, "q", 1, 30);
    check("pwrite_page0_ok", r == 1, r);
    check("page0_not_shown", m[30] == 'C' && m[2 * A_PAGE + 30] == 'C', m[30]);
    r = syscall3(SYS_msync, m, 4 * A_PAGE, MS_SYNC);
    check("msync_ok", r == 0, r);
    return finish();
}
