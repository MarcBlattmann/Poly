/* In /proc/self/maps a remapped range is a mapping of its own: same file,
 * permissions "rw-s", with the file offset of pgoff; the rest of the old
 * mapping keeps its offsets. When pages are put back in linear order the
 * pieces join into one mapping again.
 * poly-diff: syscalls=names (the /proc/self/maps text carries inode numbers)
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
static u64 hexval(const char **pp) {
    const char *p = *pp;
    u64 v = 0;
    for (;; p++) {
        if (*p >= '0' && *p <= '9') v = v * 16 + (u64)(*p - '0');
        else if (*p >= 'a' && *p <= 'f') v = v * 16 + (u64)(*p - 'a' + 10);
        else break;
    }
    *pp = p;
    return v;
}

/* The lines of /proc/self/maps inside [lo, hi), as "<pages> <perms>
 * <offset/4096>;" each, in address order, into out. Returns the count. */
static int describe(u64 lo, u64 hi, char *out) {
    static char b[65536];
    i64 fd = a_open("/proc/self/maps", O_RDONLY, 0);
    if (fd < 0) return -1;
    i64 n = 0, r;
    while (n < (i64)sizeof b - 1 && (r = a_read(fd, b + n, sizeof b - 1 - n)) > 0) n += r;
    a_close(fd);
    b[n] = 0;
    int count = 0;
    *out = 0;
    for (const char *p = b; *p;) {
        u64 start = hexval(&p);
        p++;
        u64 end = hexval(&p);
        p++;
        char perms[5] = { p[0], p[1], p[2], p[3], 0 };
        p += 5;
        u64 off = hexval(&p);
        if (start >= lo && end <= hi) {
            char *o = out + str_len(out);
            a_utoa(o, (end - start) / A_PAGE);
            str_cat2(o + str_len(o), " ", perms);
            o += str_len(o);
            *o++ = ' ';
            a_utoa(o, off / A_PAGE);
            str_cpy(o + str_len(o), ";");
            count++;
        }
        while (*p && *p != '\n') p++;
        if (*p) p++;
    }
    return count;
}

int test_main(void) {
    i64 fd = lettered_file("/dev/shm/a_remap_file_pages_maps", 8);
    /* Guard pages on both sides keep neighbours from joining the range. */
    char *g = map(0, 10 * A_PAGE, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    char *m = mapped(g) ? map(g + A_PAGE, 8 * A_PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, 0) : 0;
    check("setup", fd >= 0 && mapped(g) && m == g + A_PAGE, fd);
    if (fd < 0 || !mapped(g) || m != g + A_PAGE) return finish();
    u64 lo = (u64)m, hi = (u64)m + 8 * A_PAGE;
    char d[512];

    check("one_before", describe(lo, hi, d) == 1 && str_eq(d, "8 rw-s 0;"), 0);

    check("remap_first", rfp(m, A_PAGE, 0, 5, 0) == 0, 0);
    check("two_pieces", describe(lo, hi, d) == 2 && str_eq(d, "1 rw-s 5;7 rw-s 1;"), 0);

    check("remap_middle", rfp(m + 3 * A_PAGE, 2 * A_PAGE, 0, 0, 0) == 0, 0);
    check("four_pieces", describe(lo, hi, d) == 4 && str_eq(d, "1 rw-s 5;2 rw-s 1;2 rw-s 0;3 rw-s 5;"), 0);

    check("remap_linear", rfp(m, 8 * A_PAGE, 0, 0, 0) == 0, 0);
    check("one_again", describe(lo, hi, d) == 1 && str_eq(d, "8 rw-s 0;"), 0);

    check("remap_last_linear", rfp(m + 7 * A_PAGE, A_PAGE, 0, 7, 0) == 0, 0);
    check("still_one", describe(lo, hi, d) == 1 && str_eq(d, "8 rw-s 0;"), 0);
    return finish();
}
