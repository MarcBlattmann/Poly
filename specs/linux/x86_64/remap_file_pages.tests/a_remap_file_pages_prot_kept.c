/* The remapped pages get the protection the mapping had at addr (prot
 * itself must be 0): a read-only shared mapping stays read-only (writes
 * fault, mprotect can make it writable again since the file is open
 * read-write), a PROT_NONE one stays inaccessible.
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
    a_block_sigchld();
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_prot_kept", 4);
    char *ro = map(0, 4 * A_PAGE, PROT_READ, MAP_SHARED, fd, 0);
    char *no = map(0, 2 * A_PAGE, PROT_NONE, MAP_SHARED, fd, 0);
    check("setup", fd >= 0 && mapped(ro) && mapped(no), fd);
    if (fd < 0 || !mapped(ro) || !mapped(no)) return finish();

    i64 r = rfp(ro, A_PAGE, 0, 3, 0);
    check("ro_ok", r == 0, r);
    check("ro_reads_D", pages_are(ro, "DBCD"), ro[0]);
    check("ro_write_segv", probe_write(ro) == SIGSEGV, 0);
    check("ro_other_write_segv", probe_write(ro + A_PAGE) == SIGSEGV, 0);
    r = syscall3(SYS_mprotect, ro, A_PAGE, PROT_READ | PROT_WRITE);
    check("ro_mprotect_rw_ok", r == 0, r);
    ro[1] = 'x';
    char c = 0;
    syscall4(SYS_pread64, fd, &c, 1, 3 * A_PAGE + 1);
    check("write_reaches_file_page_3", c == 'x', c);

    r = rfp(no + A_PAGE, A_PAGE, 0, 2, 0);
    check("none_ok", r == 0, r);
    check("none_read_segv", probe_read(no + A_PAGE) == SIGSEGV, 0);
    r = syscall3(SYS_mprotect, no, 2 * A_PAGE, PROT_READ);
    check("none_mprotect_ok", r == 0, r);
    check("none_now_shows_AC", no[0] == 'A' && no[A_PAGE] == 'C', no[A_PAGE]);
    return finish();
}
