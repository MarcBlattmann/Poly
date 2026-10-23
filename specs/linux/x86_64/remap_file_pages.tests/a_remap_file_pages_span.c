/* The range may cover several mappings when they follow each other
 * without a gap, map the same file and have the same protection (and
 * other flags); their file offsets need not line up. A range that runs
 * into a gap, a mapping of another file or one with other protection is
 * EINVAL, and nothing in it is changed.
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
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_span", 8);
    i64 fd2 = lettered_file("/dev/shm/a_remap_file_pages_span_2", 2);
    char *m = map(0, 8 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && fd2 >= 0 && mapped(m), fd);
    if (fd < 0 || fd2 < 0 || !mapped(m)) return finish();

    /* Pieces made by remap_file_pages itself. */
    check("split_1", rfp(m + A_PAGE, A_PAGE, 0, 6, 0) == 0, 0);
    check("split_2", rfp(m + 3 * A_PAGE, A_PAGE, 0, 0, 0) == 0, 0);
    check("split_layout", pages_are(m, "AGCAEFGH"), m[A_PAGE]);
    i64 r = rfp(m, 5 * A_PAGE, 0, 3, 0);
    check("across_pieces_ok", r == 0, r);
    check("across_pieces_applied", pages_are(m, "DEFGHFGH"), m[0]);

    /* A separate mmap of the same file next to it, at an unrelated offset. */
    char *n = map(m + 4 * A_PAGE, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, 1 * A_PAGE);
    check("setup_same_file_neighbour", n == m + 4 * A_PAGE, 0);
    r = rfp(m + 3 * A_PAGE, 2 * A_PAGE, 0, 0, 0);
    check("same_file_neighbour_ok", r == 0, r);
    check("same_file_neighbour_applied", pages_are(m, "DEFABFGH"), m[4 * A_PAGE]);

    /* Other protection. */
    check("setup_mprotect", syscall3(SYS_mprotect, m + 5 * A_PAGE, A_PAGE, PROT_READ) == 0, 0);
    r = rfp(m + 4 * A_PAGE, 2 * A_PAGE, 0, 0, 0);
    check("other_prot_einval", r == -EINVAL, r);
    r = rfp(m + 5 * A_PAGE, A_PAGE, 0, 0, 0);
    check("inside_other_prot_ok", r == 0, r);
    check("other_prot_layout", pages_are(m, "DEFABAGH"), m[4 * A_PAGE]);
    check("restore_prot", syscall3(SYS_mprotect, m, 8 * A_PAGE, PROT_READ | PROT_WRITE) == 0, 0);

    /* Another file. */
    n = map(m + 6 * A_PAGE, A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd2, A_PAGE);
    check("setup_other_file", n == m + 6 * A_PAGE && m[6 * A_PAGE] == 'B', 0);
    r = rfp(m + 5 * A_PAGE, 2 * A_PAGE, 0, 2, 0);
    check("other_file_einval", r == -EINVAL, r);
    r = rfp(m + 6 * A_PAGE, A_PAGE, 0, 0, 0);
    check("inside_other_file_ok", r == 0, r);
    check("other_file_page", m[6 * A_PAGE] == 'A' && m[5 * A_PAGE] == 'A', m[6 * A_PAGE]);

    /* A gap. */
    check("setup_gap", syscall2(SYS_munmap, m + 2 * A_PAGE, A_PAGE) == 0, 0);
    r = rfp(m + A_PAGE, 2 * A_PAGE, 0, 0, 0);
    check("into_gap_einval", r == -EINVAL, r);
    r = rfp(m, 4 * A_PAGE, 0, 0, 0);
    check("over_gap_einval", r == -EINVAL, r);
    check("gap_unchanged", m[0] == 'D' && m[A_PAGE] == 'E' && m[3 * A_PAGE] == 'A', m[0]);
    return finish();
}
