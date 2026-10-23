/* pgoff is a 64-bit page offset into the file. pgoff plus the page count
 * wrapping past 2^64 is EINVAL; a page range that would end beyond the
 * file system's largest file offset (on tmpfs: past 2^63 - 1 bytes) is
 * EOVERFLOW. pgoff beyond the end of the file is accepted. A failed call
 * leaves the old pages mapped.
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
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_pgoff", 4);
    char *m = map(0, 4 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && mapped(m), fd);
    if (fd < 0 || !mapped(m)) return finish();

    i64 r = rfp(m, A_PAGE, 0, ~0UL, 0);
    check("max_one_page_einval", r == -EINVAL, r);
    r = rfp(m, 2 * A_PAGE, 0, ~0UL - 1, 0);
    check("wrap_two_pages_einval", r == -EINVAL, r);
    r = rfp(m, A_PAGE, 0, ~0UL - 1, 0);
    check("max_minus_1_eoverflow", r == -EOVERFLOW, r);
    r = rfp(m, A_PAGE, 0, 1UL << 63, 0);
    check("bit63_eoverflow", r == -EOVERFLOW, r);
    r = rfp(m, A_PAGE, 0, 1UL << 51, 0);
    check("bit51_eoverflow", r == -EOVERFLOW, r);
    r = rfp(m, A_PAGE, 0, (1UL << 51) - 1, 0);
    check("last_page_offset_eoverflow", r == -EOVERFLOW, r);
    r = rfp(m, 2 * A_PAGE, 0, (1UL << 51) - 2, 0);
    check("two_pages_to_end_eoverflow", r == -EOVERFLOW, r);
    check("unchanged_after_eoverflow", pages_are(m, "ABCD"), m[0]);

    r = rfp(m, A_PAGE, 0, (1UL << 51) - 2, 0);
    check("highest_one_page_ok", r == 0, r);
    r = rfp(m + A_PAGE, 2 * A_PAGE, 0, (1UL << 51) - 3, 0);
    check("highest_two_pages_ok", r == 0, r);
    r = rfp(m + 3 * A_PAGE, A_PAGE, 0, 1UL << 40, 0);
    check("far_past_eof_ok", r == 0, r);

    r = rfp(m, 4 * A_PAGE, 0, 0, 0);
    check("linear_ok", r == 0, r);
    check("linear_again", pages_are(m, "ABCD"), m[0]);
    r = rfp(m + A_PAGE, A_PAGE, 0, 4, 0);
    check("just_past_eof_ok", r == 0, r);
    r = rfp(m + 2 * A_PAGE, A_PAGE, 0, 1UL << 32, 0);
    check("pgoff_bit32_ok", r == 0, r);
    check("others_kept", m[0] == 'A' && m[3 * A_PAGE] == 'D', m[0]);
    return finish();
}
