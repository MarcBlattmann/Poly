/* size is rounded down to whole pages: 0 or less than a page is EINVAL,
 * a page and a bit remaps one page. A range that runs past the end of the
 * mapping, or wraps around the address space, is EINVAL and changes
 * nothing.
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
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_size", 8);
    char *m = map(0, 8 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && mapped(m), fd);
    if (fd < 0 || !mapped(m)) return finish();

    i64 r = rfp(m, 0, 0, 3, 0);
    check("size_0_einval", r == -EINVAL, r);
    r = rfp(m, 1, 0, 3, 0);
    check("size_1_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE - 1, 0, 3, 0);
    check("size_page_minus_1_einval", r == -EINVAL, r);
    check("unchanged_1", pages_are(m, "ABCDEFGH"), m[0]);

    r = rfp(m, A_PAGE + 1, 0, 4, 0);
    check("size_page_plus_1_ok", r == 0, r);
    check("one_page_remapped", pages_are(m, "EBCDEFGH"), m[A_PAGE]);
    r = rfp(m + 2 * A_PAGE, 3 * A_PAGE - 1, 0, 0, 0);
    check("size_3_pages_minus_1_ok", r == 0, r);
    check("two_pages_remapped", pages_are(m, "EBABEFGH"), m[4 * A_PAGE]);

    r = rfp(m + 7 * A_PAGE, 2 * A_PAGE, 0, 0, 0);
    check("past_end_einval", r == -EINVAL, r);
    r = rfp(m, 9 * A_PAGE, 0, 0, 0);
    check("whole_plus_one_einval", r == -EINVAL, r);
    r = rfp(m, 1UL << 47, 0, 0, 0);
    check("huge_einval", r == -EINVAL, r);
    r = rfp(m, (1UL << 32) + A_PAGE, 0, 0, 0);
    check("size_bit32_einval", r == -EINVAL, r);
    r = rfp(m, ~0UL, 0, 0, 0);
    check("size_max_einval", r == -EINVAL, r);
    r = rfp((void *)-(i64)A_PAGE, 2 * A_PAGE, 0, 0, 0);
    check("wrap_einval", r == -EINVAL, r);
    check("unchanged_2", pages_are(m, "EBABEFGH"), m[0]);

    r = rfp(m, 8 * A_PAGE, 0, 0, 0);
    check("exact_whole_ok", r == 0, r);
    check("linear_again", pages_are(m, "ABCDEFGH"), m[0]);
    return finish();
}
