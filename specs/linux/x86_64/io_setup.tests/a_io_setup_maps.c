/* The ring is a shared, readable and writable mapping in the caller's
 * memory: in /proc/self/maps it starts at the handle, has permissions
 * "rw-s", offset 0 and the name "/[aio] (deleted)", and its size is
 * whole pages (1 page for nr_events 1, 16 for 1000, 65 for 4096).
 * io_destroy removes it.
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

static int ends_with(const char *s, const char *suffix) {
    size_t a = str_len(s), b = str_len(suffix);
    return a >= b && str_eq(s + a - b, suffix);
}

/* The offset field (third column) of a maps line is all zeros. */
static int offset_zero(const char *line) {
    const char *p = line;
    for (int col = 0; col < 2; col++) {
        while (*p && *p != ' ') p++;
        while (*p == ' ') p++;
    }
    if (!*p) return 0;
    while (*p && *p != ' ')
        if (*p++ != '0') return 0;
    return 1;
}

static void one(const char *tag, u64 nr, u64 pages) {
    char name[64], line[256], perms[5];
    u64 ctx = 0, size = 0;
    i64 r = io_setup(nr, &ctx);
    check(str_cat2(name, tag, "_setup_ok"), r == 0, r);
    if (r != 0) return;
    int found = find_map(ctx, &size, perms, line, sizeof line);
    check(str_cat2(name, tag, "_mapped_at_handle"), found, 0);
    if (found) {
        check(str_cat2(name, tag, "_size"), size == pages * 4096, (i64)(size / 4096));
        check(str_cat2(name, tag, "_perms_rw_s"), str_eq(perms, "rw-s"), 0);
        check(str_cat2(name, tag, "_offset_0"), offset_zero(line), 0);
        check(str_cat2(name, tag, "_name"), ends_with(line, " /[aio] (deleted)"), 0);
    }
    check(str_cat2(name, tag, "_destroy_ok"), io_destroy(ctx) == 0, 0);
    found = find_map(ctx, &size, perms, line, sizeof line);
    check(str_cat2(name, tag, "_unmapped_after_destroy"), !found, 0);
}

int test_main(void) {
    check("no_aio_at_start", count_aio() == 0, count_aio());
    one("nr1", 1, 1);
    one("nr1000", 1000, 16);
    one("nr4096", 4096, 65);

    u64 a = 0, b = 0;
    io_setup(1, &a);
    io_setup(1, &b);
    check("two_contexts_two_maps", count_aio() == 2, count_aio());
    io_destroy(a);
    check("one_left", count_aio() == 1, count_aio());
    io_destroy(b);
    check("none_left", count_aio() == 0, count_aio());
    return finish();
}
