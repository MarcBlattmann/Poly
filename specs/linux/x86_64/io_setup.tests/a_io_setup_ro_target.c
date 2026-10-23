/* *ctx_idp must also be writable: a readable page holding 0 that cannot
 * be written gives EFAULT, and the context is not left behind (no [aio]
 * mapping remains). A readable value that is not 0 is EINVAL, before
 * any write; the nr_events checks come before the write too.
 * poly-diff: syscalls=names (maps contents and read sizes depend on addresses and inode numbers)
 */
#include "poly_test.h"
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_io_setup 206
#define SYS_io_destroy 207

static i64 io_setup(u64 nr, void *ctxp) { return syscall2(SYS_io_setup, nr, ctxp); }
static i64 io_destroy(u64 ctx) { return syscall1(SYS_io_destroy, ctx); }

static char maps_buf[65536];

static u64 hexval(const char **pp) {
    const char *p = *pp;
    u64 v = 0;
    for (;; p++) {
        char c = *p;
        if (c >= '0' && c <= '9') v = v * 16 + (u64)(c - '0');
        else if (c >= 'a' && c <= 'f') v = v * 16 + (u64)(c - 'a' + 10);
        else break;
    }
    *pp = p;
    return v;
}

static void read_maps(void) {
    i64 fd = a_open("/proc/self/maps", 0, 0);
    i64 t = 0, n;
    if (fd < 0) {
        maps_buf[0] = 0;
        return;
    }
    while (t < (i64)sizeof maps_buf - 1 && (n = a_read(fd, maps_buf + t, sizeof maps_buf - 1 - t)) > 0) t += n;
    a_close(fd);
    maps_buf[t] = 0;
}

static int has_aio_name(const char *p, const char *e) {
    for (; p + 5 <= e; p++)
        if (p[0] == '[' && p[1] == 'a' && p[2] == 'i' && p[3] == 'o' && p[4] == ']') return 1;
    return 0;
}

/* Number of mappings named [aio]. */
static __attribute__((unused)) int count_aio(void) {
    read_maps();
    int n = 0;
    const char *p = maps_buf;
    while (*p) {
        const char *e = p;
        while (*e && *e != '\n') e++;
        n += has_aio_name(p, e);
        p = *e ? e + 1 : e;
    }
    return n;
}

/* Find the mapping that starts at addr: its size, permission string and
 * whole line. Returns 0 if there is none. */
static __attribute__((unused)) int find_map(u64 addr, u64 *size, char perms[5], char *line, int cap) {
    read_maps();
    const char *p = maps_buf;
    while (*p) {
        const char *e = p;
        while (*e && *e != '\n') e++;
        const char *q = p;
        u64 lo = hexval(&q);
        if (*q == '-') {
            q++;
            u64 hi = hexval(&q);
            if (lo == addr) {
                *size = hi - lo;
                q++;
                for (int i = 0; i < 4; i++) perms[i] = q[i];
                perms[4] = 0;
                int n = (int)(e - p);
                if (n > cap - 1) n = cap - 1;
                memcpy(line, p, (size_t)n);
                line[n] = 0;
                return 1;
            }
        }
        p = *e ? e + 1 : e;
    }
    return 0;
}

static const u64 zero_ro = 0;
static const u64 one_ro = 1;

int test_main(void) {
    i64 r = io_setup(5, (void *)&zero_ro);
    check("rodata_efault", r == -EFAULT, r);
    check("rodata_no_mapping_left", count_aio() == 0, count_aio());

    u64 *page = (u64 *)syscall6(SYS_mmap, 0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    check("mmap_ok", (i64)page > 0, (i64)page);
    r = io_setup(1000, page);
    check("ro_page_efault", r == -EFAULT, r);
    check("ro_page_still_zero", *page == 0, (i64)*page);
    check("ro_page_no_mapping_left", count_aio() == 0, count_aio());

    r = io_setup(1, (void *)&one_ro);
    check("ro_nonzero_einval", r == -EINVAL, r);
    r = io_setup(0, (void *)&zero_ro);
    check("ro_nr0_einval", r == -EINVAL, r);
    r = io_setup(0x400001, (void *)&zero_ro);
    check("ro_too_large_einval", r == -EINVAL, r);
    r = io_setup(65537, (void *)&zero_ro);
    check("ro_over_limit_eagain", r == -EAGAIN, r);

    u64 ctx = 0;
    r = io_setup(1000, &ctx);
    check("writable_after_failed_ok", r == 0, r);
    check("one_mapping", count_aio() == 1, count_aio());
    if (r == 0) check("destroy_ok", io_destroy(ctx) == 0, 0);
    return finish();
}
